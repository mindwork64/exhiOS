#![no_std]

//! Allocation-free VFS contracts and a bounded in-memory filesystem.
//!
//! Node handles carry generation counters, so a removed slot cannot revive a
//! stale handle when the slot is reused. The first RAMFS has fixed capacities
//! and predictable failure modes suitable for the initial kernel filesystem.

pub const MAX_NAME_BYTES: usize = 31;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NodeId {
    slot: usize,
    generation: u64,
}
impl NodeId {
    pub const fn slot(self) -> usize { self.slot }
    pub const fn generation(self) -> u64 { self.generation }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind { File, Directory }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Name {
    bytes: [u8; MAX_NAME_BYTES],
    length: u8,
}
impl Name {
    pub fn new(value: &str) -> Result<Self, VfsError> {
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > MAX_NAME_BYTES
            || value == "."
            || value == ".."
            || bytes.iter().any(|byte| {
                *byte == b'/' || *byte == 0 || *byte < 0x20 || *byte == 0x7F
            })
        {
            return Err(VfsError::InvalidName);
        }
        let mut name = Self { bytes: [0; MAX_NAME_BYTES], length: bytes.len() as u8 };
        name.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(name)
    }
    const fn root() -> Self {
        Self { bytes: [0; MAX_NAME_BYTES], length: 0 }
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..usize::from(self.length)])
            .expect("Name stores only validated UTF-8")
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.length)]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Metadata {
    pub id: NodeId,
    pub parent: NodeId,
    pub name: Name,
    pub kind: NodeKind,
    pub size: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub id: NodeId,
    pub name: Name,
    pub kind: NodeKind,
    pub size: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VfsError {
    InvalidCapacity,
    InvalidName,
    NotFound,
    AlreadyExists,
    NotDirectory,
    IsDirectory,
    DirectoryNotEmpty,
    NoSpace,
    StaleNode,
    RootRemovalDenied,
    OffsetOverflow,
    FileTooLarge,
    GenerationExhausted,
}
impl VfsError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidCapacity => "invalid filesystem capacity",
            Self::InvalidName => "invalid node name",
            Self::NotFound => "node not found",
            Self::AlreadyExists => "node already exists",
            Self::NotDirectory => "parent is not a directory",
            Self::IsDirectory => "operation requires a file",
            Self::DirectoryNotEmpty => "directory is not empty",
            Self::NoSpace => "filesystem node table is full",
            Self::StaleNode => "stale or invalid node handle",
            Self::RootRemovalDenied => "root directory cannot be removed",
            Self::OffsetOverflow => "file offset arithmetic overflow",
            Self::FileTooLarge => "file capacity exceeded",
            Self::GenerationExhausted => "node generation counter exhausted",
        }
    }
}

/// Backend contract used by kernel services and future persistent filesystems.
/// No allocator or architecture dependencies are required.
pub trait FileSystem {
    fn root(&self) -> NodeId;
    fn lookup(&self, parent: NodeId, name: &str) -> Result<NodeId, VfsError>;
    fn metadata(&self, id: NodeId) -> Result<Metadata, VfsError>;
    fn create_file(&mut self, parent: NodeId, name: &str) -> Result<NodeId, VfsError>;
    fn create_directory(&mut self, parent: NodeId, name: &str) -> Result<NodeId, VfsError>;
    fn read(&self, file: NodeId, offset: usize, destination: &mut [u8]) -> Result<usize, VfsError>;
    fn write(&mut self, file: NodeId, offset: usize, source: &[u8]) -> Result<usize, VfsError>;
    fn remove(&mut self, id: NodeId) -> Result<(), VfsError>;
    fn read_directory(
        &self,
        directory: NodeId,
        cursor: &mut usize,
        entries: &mut [DirectoryEntry],
    ) -> Result<usize, VfsError>;
}

#[derive(Clone, Copy)]
struct Node<const FILE_CAPACITY: usize> {
    id: NodeId,
    parent: NodeId,
    name: Name,
    kind: NodeKind,
    size: usize,
    data: [u8; FILE_CAPACITY],
}

/// Bounded RAM filesystem with a fixed node table and fixed per-file capacity.
/// Directory nodes also carry the fixed data buffer in this first version.
pub struct RamFs<const NODE_COUNT: usize, const FILE_CAPACITY: usize> {
    nodes: [Option<Node<FILE_CAPACITY>>; NODE_COUNT],
    generations: [u64; NODE_COUNT],
    root: NodeId,
}

impl<const NODE_COUNT: usize, const FILE_CAPACITY: usize> RamFs<NODE_COUNT, FILE_CAPACITY> {
    pub fn new() -> Result<Self, VfsError> {
        if NODE_COUNT == 0 || FILE_CAPACITY == 0 {
            return Err(VfsError::InvalidCapacity);
        }
        let root = NodeId { slot: 0, generation: 1 };
        let root_node = Node {
            id: root,
            parent: root,
            name: Name::root(),
            kind: NodeKind::Directory,
            size: 0,
            data: [0; FILE_CAPACITY],
        };
        let mut nodes = core::array::from_fn(|_| None);
        nodes[0] = Some(root_node);
        let mut generations = [0; NODE_COUNT];
        generations[0] = 1;
        Ok(Self { nodes, generations, root })
    }

    fn node(&self, id: NodeId) -> Result<&Node<FILE_CAPACITY>, VfsError> {
        self.nodes
            .get(id.slot)
            .and_then(Option::as_ref)
            .filter(|node| node.id == id)
            .ok_or(VfsError::StaleNode)
    }

    fn node_mut(&mut self, id: NodeId) -> Result<&mut Node<FILE_CAPACITY>, VfsError> {
        self.nodes
            .get_mut(id.slot)
            .and_then(Option::as_mut)
            .filter(|node| node.id == id)
            .ok_or(VfsError::StaleNode)
    }

    fn create_node(
        &mut self,
        parent: NodeId,
        name: &str,
        kind: NodeKind,
    ) -> Result<NodeId, VfsError> {
        let name = Name::new(name)?;
        if self.node(parent)?.kind != NodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        if self.nodes.iter().flatten().any(|node| {
            node.parent == parent && node.name == name
        }) {
            return Err(VfsError::AlreadyExists);
        }
        let slot = self.nodes.iter().position(Option::is_none).ok_or(VfsError::NoSpace)?;
        let generation = self.generations[slot]
            .checked_add(1)
            .ok_or(VfsError::GenerationExhausted)?;
        let id = NodeId { slot, generation };
        self.generations[slot] = generation;
        self.nodes[slot] = Some(Node {
            id,
            parent,
            name,
            kind,
            size: 0,
            data: [0; FILE_CAPACITY],
        });
        Ok(id)
    }

    fn directory_has_children(&self, directory: NodeId) -> bool {
        self.nodes
            .iter()
            .flatten()
            .any(|node| node.parent == directory && node.id != directory)
    }
}

impl<const NODE_COUNT: usize, const FILE_CAPACITY: usize> FileSystem
    for RamFs<NODE_COUNT, FILE_CAPACITY>
{
    fn root(&self) -> NodeId { self.root }

    fn lookup(&self, parent: NodeId, name: &str) -> Result<NodeId, VfsError> {
        if self.node(parent)?.kind != NodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        let name = Name::new(name)?;
        self.nodes.iter().flatten()
            .find(|node| node.parent == parent && node.name == name)
            .map(|node| node.id)
            .ok_or(VfsError::NotFound)
    }

    fn metadata(&self, id: NodeId) -> Result<Metadata, VfsError> {
        let node = self.node(id)?;
        Ok(Metadata {
            id: node.id,
            parent: node.parent,
            name: node.name,
            kind: node.kind,
            size: node.size,
        })
    }

    fn create_file(&mut self, parent: NodeId, name: &str) -> Result<NodeId, VfsError> {
        self.create_node(parent, name, NodeKind::File)
    }

    fn create_directory(&mut self, parent: NodeId, name: &str) -> Result<NodeId, VfsError> {
        self.create_node(parent, name, NodeKind::Directory)
    }

    fn read(&self, file: NodeId, offset: usize, destination: &mut [u8]) -> Result<usize, VfsError> {
        let node = self.node(file)?;
        if node.kind == NodeKind::Directory {
            return Err(VfsError::IsDirectory);
        }
        if offset >= node.size || destination.is_empty() {
            return Ok(0);
        }
        let count = destination.len().min(node.size - offset);
        destination[..count].copy_from_slice(&node.data[offset..offset + count]);
        Ok(count)
    }

    fn write(&mut self, file: NodeId, offset: usize, source: &[u8]) -> Result<usize, VfsError> {
        let end = offset.checked_add(source.len()).ok_or(VfsError::OffsetOverflow)?;
        if end > FILE_CAPACITY || offset > FILE_CAPACITY {
            return Err(VfsError::FileTooLarge);
        }
        let node = self.node_mut(file)?;
        if node.kind == NodeKind::Directory {
            return Err(VfsError::IsDirectory);
        }
        if source.is_empty() {
            return Ok(0);
        }
        if offset > node.size {
            node.data[node.size..offset].fill(0);
        }
        node.data[offset..end].copy_from_slice(source);
        node.size = node.size.max(end);
        Ok(source.len())
    }

    fn remove(&mut self, id: NodeId) -> Result<(), VfsError> {
        let node = self.node(id)?;
        if id == self.root {
            return Err(VfsError::RootRemovalDenied);
        }
        if node.kind == NodeKind::Directory && self.directory_has_children(id) {
            return Err(VfsError::DirectoryNotEmpty);
        }
        self.nodes[id.slot] = None;
        Ok(())
    }

    fn read_directory(
        &self,
        directory: NodeId,
        cursor: &mut usize,
        entries: &mut [DirectoryEntry],
    ) -> Result<usize, VfsError> {
        if self.node(directory)?.kind != NodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        if entries.is_empty() {
            return Ok(0);
        }
        let mut written = 0;
        while *cursor < NODE_COUNT && written < entries.len() {
            let slot = *cursor;
            *cursor += 1;
            if let Some(node) = self.nodes[slot].filter(|node| node.parent == directory) {
                if node.id != directory {
                    entries[written] = DirectoryEntry {
                        id: node.id,
                        name: node.name,
                        kind: node.kind,
                        size: node.size,
                    };
                    written += 1;
                }
            }
        }
        Ok(written)
    }
}

/// Resolves an absolute or base-relative path without allocation. Repeated
/// separators and the current-directory component are ignored; parent
/// components stop at the filesystem root.
pub fn resolve_path<F: FileSystem + ?Sized>(
    filesystem: &F,
    base: NodeId,
    path: &str,
) -> Result<NodeId, VfsError> {
    let mut current = if path.starts_with('/') {
        filesystem.root()
    } else {
        if filesystem.metadata(base)?.kind != NodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        base
    };
    for component in path.split('/').filter(|part| !part.is_empty()) {
        match component {
            "." => {}
            ".." => current = filesystem.metadata(current)?.parent,
            name => current = filesystem.lookup(current, name)?,
        }
    }
    if path.ends_with('/') && filesystem.metadata(current)?.kind != NodeKind::Directory {
        return Err(VfsError::NotDirectory);
    }
    Ok(current)
}

/// Creates a file at an absolute or base-relative path.
pub fn create_file_path<F: FileSystem + ?Sized>(
    filesystem: &mut F,
    base: NodeId,
    path: &str,
) -> Result<NodeId, VfsError> {
    let (parent_path, name) = split_parent(path)?;
    let parent = resolve_path(filesystem, base, parent_path)?;
    filesystem.create_file(parent, name)
}

/// Creates a directory at an absolute or base-relative path.
pub fn create_directory_path<F: FileSystem + ?Sized>(
    filesystem: &mut F,
    base: NodeId,
    path: &str,
) -> Result<NodeId, VfsError> {
    let (parent_path, name) = split_parent(path)?;
    let parent = resolve_path(filesystem, base, parent_path)?;
    filesystem.create_directory(parent, name)
}

fn split_parent(path: &str) -> Result<(&str, &str), VfsError> {
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return Err(VfsError::InvalidName);
    }
    match path.rsplit_once('/') {
        Some(("", name)) => Ok(("/", name)),
        Some((parent, name)) => Ok((parent, name)),
        None => Ok((".", path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestFs = RamFs<8, 16>;

    fn fs() -> TestFs { TestFs::new().unwrap() }

    #[test]
    fn creates_and_resolves_nested_paths() {
        let mut filesystem = fs();
        let root = filesystem.root();
        let etc = create_directory_path(&mut filesystem, root, "/etc").unwrap();
        let config = create_file_path(&mut filesystem, root, "/etc/exhi.conf").unwrap();
        assert_eq!(resolve_path(&filesystem, root, "/etc//./exhi.conf").unwrap(), config);
        assert_eq!(resolve_path(&filesystem, etc, "../etc/exhi.conf").unwrap(), config);
        assert_eq!(filesystem.metadata(config).unwrap().kind, NodeKind::File);
        assert_eq!(filesystem.lookup(root, "missing"), Err(VfsError::NotFound));
    }

    #[test]
    fn reads_and_writes_binary_data_with_sparse_zero_fill() {
        let mut filesystem = fs();
        let file = filesystem.create_file(filesystem.root(), "data.bin").unwrap();
        assert_eq!(filesystem.write(file, 2, &[0x00, 0xFF, 0x80]), Ok(3));
        let mut bytes = [0xAA; 8];
        assert_eq!(filesystem.read(file, 0, &mut bytes), Ok(5));
        assert_eq!(&bytes[..5], &[0, 0, 0x00, 0xFF, 0x80]);
        assert_eq!(filesystem.metadata(file).unwrap().size, 5);
        assert_eq!(filesystem.read(file, 5, &mut bytes), Ok(0));
    }

    #[test]
    fn oversized_write_is_rejected_without_partial_modification() {
        let mut filesystem = fs();
        let file = filesystem.create_file(filesystem.root(), "data").unwrap();
        filesystem.write(file, 0, b"stable").unwrap();
        assert_eq!(filesystem.write(file, 14, b"four"), Err(VfsError::FileTooLarge));
        let mut bytes = [0; 8];
        assert_eq!(filesystem.read(file, 0, &mut bytes), Ok(6));
        assert_eq!(&bytes[..6], b"stable");
        assert_eq!(filesystem.metadata(file).unwrap().size, 6);
    }

    #[test]
    fn directory_reads_support_cursor_pagination() {
        let mut filesystem = fs();
        let root = filesystem.root();
        filesystem.create_file(root, "one").unwrap();
        filesystem.create_directory(root, "two").unwrap();
        filesystem.create_file(root, "three").unwrap();
        let mut cursor = 0;
        let placeholder = DirectoryEntry {
            id: root,
            name: Name::root(),
            kind: NodeKind::Directory,
            size: 0,
        };
        let mut first = [placeholder; 2];
        assert_eq!(filesystem.read_directory(root, &mut cursor, &mut first), Ok(2));
        assert_eq!(first[0].name.as_str(), "one");
        assert_eq!(first[1].name.as_str(), "two");
        let mut second = [first[0]; 2];
        assert_eq!(filesystem.read_directory(root, &mut cursor, &mut second), Ok(1));
        assert_eq!(second[0].name.as_str(), "three");
        assert_eq!(filesystem.read_directory(root, &mut cursor, &mut second), Ok(0));
    }

    #[test]
    fn rejects_wrong_node_kinds_and_nonempty_directory_removal() {
        let mut filesystem = fs();
        let root = filesystem.root();
        let file = filesystem.create_file(root, "file").unwrap();
        assert_eq!(filesystem.create_file(file, "child"), Err(VfsError::NotDirectory));
        let mut cursor = 0;
        assert_eq!(filesystem.read_directory(file, &mut cursor, &mut []), Err(VfsError::NotDirectory));
        let directory = filesystem.create_directory(root, "dir").unwrap();
        filesystem.create_file(directory, "child").unwrap();
        assert_eq!(filesystem.remove(directory), Err(VfsError::DirectoryNotEmpty));
        assert_eq!(filesystem.remove(root), Err(VfsError::RootRemovalDenied));
        assert_eq!(filesystem.read(directory, 0, &mut []), Err(VfsError::IsDirectory));
    }

    #[test]
    fn reused_slots_never_revive_stale_handles() {
        let mut filesystem = fs();
        let root = filesystem.root();
        let stale = filesystem.create_file(root, "old").unwrap();
        filesystem.remove(stale).unwrap();
        let fresh = filesystem.create_file(root, "new").unwrap();
        assert_eq!(stale.slot(), fresh.slot());
        assert_ne!(stale.generation(), fresh.generation());
        assert_eq!(filesystem.metadata(stale), Err(VfsError::StaleNode));
        assert_eq!(filesystem.metadata(fresh).unwrap().name.as_str(), "new");
    }

    #[test]
    fn node_capacity_and_name_validation_are_bounded() {
        let mut filesystem = RamFs::<2, 8>::new().unwrap();
        let root = filesystem.root();
        filesystem.create_file(root, "ok").unwrap();
        assert_eq!(filesystem.create_file(root, "second"), Err(VfsError::NoSpace));
        assert_eq!(filesystem.create_file(root, "../bad"), Err(VfsError::InvalidName));
        let long_name = "x".repeat(MAX_NAME_BYTES + 1);
        assert_eq!(filesystem.create_file(root, &long_name), Err(VfsError::InvalidName));
    }

    #[test]
    fn zero_length_operations_are_safe() {
        let mut filesystem = fs();
        let file = filesystem.create_file(filesystem.root(), "empty").unwrap();
        assert_eq!(filesystem.write(file, 0, &[]), Ok(0));
        assert_eq!(filesystem.read(file, 0, &mut []), Ok(0));
        assert_eq!(filesystem.metadata(file).unwrap().size, 0);
    }
}
