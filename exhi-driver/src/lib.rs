#![no_std]

/// Stable identifier for each built-in kernel driver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum DriverId {
    SerialCom1 = 1,
    PitChannel0 = 2,
    Ps2Keyboard = 3,
}

/// Broad function served by a driver. This is deliberately not a device-bus
/// taxonomy; it is a small kernel inventory useful to boot diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DriverClass {
    Serial,
    Clock,
    Input,
}

/// Runtime lifecycle tracked by the registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DriverState {
    Registered,
    Initialized,
    Failed,
}

/// Static driver metadata. Descriptors contain no heap-owned strings or
/// mutable state and can therefore live in driver modules as constants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DriverDescriptor {
    id: DriverId,
    class: DriverClass,
    name: &'static str,
}

impl DriverDescriptor {
    pub const fn new(id: DriverId, class: DriverClass, name: &'static str) -> Self {
        Self { id, class, name }
    }

    pub const fn id(self) -> DriverId {
        self.id
    }

    pub const fn class(self) -> DriverClass {
        self.class
    }

    pub const fn name(self) -> &'static str {
        self.name
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DriverEntry {
    pub descriptor: DriverDescriptor,
    pub state: DriverState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryError {
    Full,
    DuplicateId,
    NotFound,
    InvalidStateTransition,
}

/// Fixed-capacity, allocation-free inventory of registered built-in drivers.
///
/// The registry records metadata and lifecycle; it intentionally does not call
/// driver initialization functions. Hardware initialization order remains
/// explicit in kernel startup, where IDT/PIC/device dependencies are visible.
pub struct DriverRegistry<const CAPACITY: usize> {
    entries: [Option<DriverEntry>; CAPACITY],
    len: usize,
}

impl<const CAPACITY: usize> DriverRegistry<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            entries: [None; CAPACITY],
            len: 0,
        }
    }

    pub fn register(&mut self, descriptor: DriverDescriptor) -> Result<(), RegistryError> {
        if self.get(descriptor.id()).is_some() {
            return Err(RegistryError::DuplicateId);
        }
        if self.len == CAPACITY {
            return Err(RegistryError::Full);
        }

        self.entries[self.len] = Some(DriverEntry {
            descriptor,
            state: DriverState::Registered,
        });
        self.len += 1;
        Ok(())
    }

    /// Applies a valid lifecycle transition. Repeating the current state is
    /// idempotent; Failed is terminal for this boot and cannot be reset.
    pub fn set_state(&mut self, id: DriverId, state: DriverState) -> Result<(), RegistryError> {
        let Some(index) = self.entries[..self.len]
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.descriptor.id() == id))
        else {
            return Err(RegistryError::NotFound);
        };

        let entry = self.entries[index].as_mut().expect("located registry entry");
        if entry.state == state {
            return Ok(());
        }

        let allowed = matches!(
            (entry.state, state),
            (DriverState::Registered, DriverState::Initialized)
                | (DriverState::Registered, DriverState::Failed)
                | (DriverState::Initialized, DriverState::Failed)
        );
        if !allowed {
            return Err(RegistryError::InvalidStateTransition);
        }

        entry.state = state;
        Ok(())
    }

    pub fn get(&self, id: DriverId) -> Option<DriverEntry> {
        self.entries[..self.len]
            .iter()
            .flatten()
            .copied()
            .find(|entry| entry.descriptor.id() == id)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub const fn capacity(&self) -> usize {
        CAPACITY
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn initialized_count(&self) -> usize {
        self.iter()
            .filter(|entry| entry.state == DriverState::Initialized)
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.iter()
            .filter(|entry| entry.state == DriverState::Failed)
            .count()
    }

    pub fn iter(&self) -> impl Iterator<Item = &DriverEntry> {
        self.entries[..self.len].iter().filter_map(Option::as_ref)
    }
}

impl<const CAPACITY: usize> Default for DriverRegistry<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERIAL: DriverDescriptor =
        DriverDescriptor::new(DriverId::SerialCom1, DriverClass::Serial, "COM1");
    const PIT: DriverDescriptor =
        DriverDescriptor::new(DriverId::PitChannel0, DriverClass::Clock, "PIT channel 0");
    const KEYBOARD: DriverDescriptor =
        DriverDescriptor::new(DriverId::Ps2Keyboard, DriverClass::Input, "PS/2 keyboard");

    #[test]
    fn registers_descriptors_in_insertion_order() {
        let mut registry = DriverRegistry::<4>::new();
        registry.register(SERIAL).unwrap();
        registry.register(PIT).unwrap();
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.get(DriverId::SerialCom1).unwrap().descriptor.name(), "COM1");
        let mut entries = registry.iter();
        assert_eq!(entries.next().map(|entry| entry.descriptor.id()), Some(DriverId::SerialCom1));
        assert_eq!(entries.next().map(|entry| entry.descriptor.id()), Some(DriverId::PitChannel0));
        assert!(entries.next().is_none());
    }

    #[test]
    fn new_entries_start_registered() {
        let mut registry = DriverRegistry::<2>::new();
        registry.register(KEYBOARD).unwrap();
        assert_eq!(registry.get(DriverId::Ps2Keyboard).unwrap().state, DriverState::Registered);
    }

    #[test]
    fn rejects_duplicate_identifiers() {
        let mut registry = DriverRegistry::<2>::new();
        registry.register(SERIAL).unwrap();
        assert_eq!(registry.register(SERIAL), Err(RegistryError::DuplicateId));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn rejects_capacity_overflow_without_mutating_registry() {
        let mut registry = DriverRegistry::<1>::new();
        registry.register(SERIAL).unwrap();
        assert_eq!(registry.register(PIT), Err(RegistryError::Full));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn supports_registered_to_initialized_transition() {
        let mut registry = DriverRegistry::<2>::new();
        registry.register(PIT).unwrap();
        registry.set_state(DriverId::PitChannel0, DriverState::Initialized).unwrap();
        assert_eq!(registry.initialized_count(), 1);
    }

    #[test]
    fn supports_failure_transition_and_counts_failures() {
        let mut registry = DriverRegistry::<2>::new();
        registry.register(KEYBOARD).unwrap();
        registry.set_state(DriverId::Ps2Keyboard, DriverState::Failed).unwrap();
        assert_eq!(registry.failed_count(), 1);
        assert_eq!(
            registry.set_state(DriverId::Ps2Keyboard, DriverState::Registered),
            Err(RegistryError::InvalidStateTransition)
        );
    }

    #[test]
    fn rejects_unknown_identifiers_and_invalid_transitions() {
        let mut registry = DriverRegistry::<2>::new();
        assert_eq!(
            registry.set_state(DriverId::PitChannel0, DriverState::Initialized),
            Err(RegistryError::NotFound)
        );
        registry.register(SERIAL).unwrap();
        assert_eq!(
            registry.set_state(DriverId::SerialCom1, DriverState::Registered),
            Ok(())
        );
        assert_eq!(
            registry.set_state(DriverId::SerialCom1, DriverState::Registered),
            Ok(())
        );
    }

    #[test]
    fn reports_capacity_and_empty_state() {
        let registry = DriverRegistry::<3>::new();
        assert!(registry.is_empty());
        assert_eq!(registry.capacity(), 3);
        assert_eq!(registry.initialized_count(), 0);
    }
}
