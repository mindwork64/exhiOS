#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
cd "$ROOT_DIR"

if ! command -v rustup >/dev/null 2>&1; then
    echo "rustup is required. Install the official Rust toolchain first." >&2
    exit 1
fi

if ! command -v readelf >/dev/null 2>&1; then
    echo "readelf is required to verify the kernel image type. Install binutils." >&2
    exit 1
fi

if [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" && "${EXHIOS_QEMU_EXIT_TEST:-0}" == "1" ]]; then
    echo "Choose either EXHIOS_EXCEPTION_TEST or EXHIOS_QEMU_EXIT_TEST, not both." >&2
    exit 1
fi
if [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] && {
    [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] || [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" == "1" ]];
}; then
    echo "EXHIOS_SERIAL_RX_TEST cannot be combined with exception or debug-exit tests." >&2
    exit 1
fi

if [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_TIMER_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with exception or serial RX tests." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_KEYBOARD_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with exception, serial RX or timer tests." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_CONTEXT_SWITCH_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_CONTEXT_SWITCH_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with other smoke-test modes." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_TASK_ACCOUNTING_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_CONTEXT_SWITCH_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_TASK_ACCOUNTING_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with other smoke-test modes." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_TIMER_PREEMPTION_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_CONTEXT_SWITCH_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TASK_ACCOUNTING_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_PREEMPTIVE_TASK_EXIT_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_TIMER_PREEMPTION_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with other smoke-test modes." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_PREEMPTIVE_TASK_EXIT_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_CONTEXT_SWITCH_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TASK_ACCOUNTING_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_PREEMPTION_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_PREEMPTIVE_TASK_EXIT_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with other smoke-test modes." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_USER_TASK_LIFECYCLE_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_CONTEXT_SWITCH_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TASK_ACCOUNTING_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_PREEMPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_PREEMPTIVE_TASK_EXIT_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_IDLE_WAKEUP_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_RING3_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_RING3_SUPERVISOR_FAULT_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_RING3_SYSCALL_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_USER_POINTER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_USER_COPY_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_USER_TASK_LIFECYCLE_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with other smoke-test modes." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_USER_COPY_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_CONTEXT_SWITCH_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TASK_ACCOUNTING_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_PREEMPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_PREEMPTIVE_TASK_EXIT_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_IDLE_WAKEUP_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_RING3_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_RING3_SUPERVISOR_FAULT_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_RING3_SYSCALL_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_USER_POINTER_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_USER_COPY_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with other smoke-test modes." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_USER_POINTER_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_CONTEXT_SWITCH_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TASK_ACCOUNTING_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_PREEMPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_PREEMPTIVE_TASK_EXIT_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_IDLE_WAKEUP_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_RING3_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_RING3_SUPERVISOR_FAULT_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_RING3_SYSCALL_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_USER_POINTER_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with other smoke-test modes." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_RING3_SYSCALL_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_CONTEXT_SWITCH_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TASK_ACCOUNTING_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_PREEMPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_PREEMPTIVE_TASK_EXIT_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_IDLE_WAKEUP_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_RING3_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_RING3_SUPERVISOR_FAULT_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_RING3_SYSCALL_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with other smoke-test modes." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_RING3_SUPERVISOR_FAULT_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_CONTEXT_SWITCH_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TASK_ACCOUNTING_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_PREEMPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_PREEMPTIVE_TASK_EXIT_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_IDLE_WAKEUP_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_RING3_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_RING3_SUPERVISOR_FAULT_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with other smoke-test modes." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_RING3_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_CONTEXT_SWITCH_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TASK_ACCOUNTING_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_PREEMPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_PREEMPTIVE_TASK_EXIT_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_IDLE_WAKEUP_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_RING3_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with other smoke-test modes." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_IDLE_WAKEUP_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_CONTEXT_SWITCH_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TASK_ACCOUNTING_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_PREEMPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_PREEMPTIVE_TASK_EXIT_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_IDLE_WAKEUP_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with other smoke-test modes." >&2
        exit 1
    fi
fi

if [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]] ||
       [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]] ||
       [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]]; then
        echo "EXHIOS_DRIVER_REGISTRY_TEST requires EXHIOS_QEMU_EXIT_TEST and cannot be combined with other smoke-test modes." >&2
        exit 1
    fi
fi

rustup target add x86_64-unknown-uefi x86_64-unknown-none

cargo test -p exhi-abi
cargo test -p exhi-elf
cargo test -p exhi-memory
cargo test -p exhi-console
cargo build -p exhi-boot --target x86_64-unknown-uefi
cargo test -p libexhi
INIT_FEATURE_ARGS=()
if [[ "${EXHIOS_PROCESS_RUN_TEST:-0}" == "1" ]]; then
    INIT_FEATURE_ARGS+=(--features qemu-smoke-test)
fi
cargo build -p exhi-init --target x86_64-unknown-none "${INIT_FEATURE_ARGS[@]}"

INIT_ELF="$ROOT_DIR/target/x86_64-unknown-none/debug/exhi-init"
echo
echo "Verifying first user process ELF type..."
LC_ALL=C readelf -h "$INIT_ELF"
if ! LC_ALL=C readelf -h "$INIT_ELF" | grep -Eq 'Type:[[:space:]]+EXEC[[:space:]]+\(Executable file\)'; then
    echo "ERROR: exhi-init must be ET_EXEC, not PIE/ET_DYN." >&2
    exit 1
fi

KERNEL_FEATURES=()
if [[ "${EXHIOS_EXCEPTION_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(exception-smoke-test)
    echo "Building the kernel with the optional INT3 exception smoke test."
fi
if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(qemu-debug-exit-smoke-test)
    echo "Building the kernel with the QEMU debug-exit smoke test."
fi
if [[ "${EXHIOS_SERIAL_RX_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(serial-rx-smoke-test)
    echo "Building the kernel with the interactive COM1 RX smoke test."
fi
if [[ "${EXHIOS_TIMER_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(timer-smoke-test)
    echo "Building the kernel with the PIT IRQ0 timer smoke test."
fi

if [[ "${EXHIOS_KEYBOARD_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(keyboard-smoke-test)
    echo "Building the kernel with the interactive PS/2 keyboard smoke test."
fi

if [[ "${EXHIOS_CONTEXT_SWITCH_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(context-switch-smoke-test)
    echo "Building the kernel with the x86-64 cooperative context-switch smoke test."
fi

if [[ "${EXHIOS_TASK_ACCOUNTING_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(task-accounting-smoke-test)
    echo "Building the kernel with PIT-backed cooperative task runtime accounting."
fi

if [[ "${EXHIOS_TIMER_PREEMPTION_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(timer-preemption-smoke-test)
    echo "Building the kernel with hardware-interrupt task preemption."
fi

if [[ "${EXHIOS_PREEMPTIVE_TASK_EXIT_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(preemptive-task-exit-smoke-test)
    echo "Building the kernel with preemptive task termination support."
fi

if [[ "${EXHIOS_IDLE_WAKEUP_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(idle-wakeup-smoke-test)
    echo "Building the kernel with idle-context and timed task wake-up support."
fi

if [[ "${EXHIOS_RING3_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(ring3-smoke-test)
    echo "Building the kernel with the Ring 3 entry smoke test."
fi

if [[ "${EXHIOS_RING3_SUPERVISOR_FAULT_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(ring3-supervisor-fault-smoke-test)
    echo "Building the kernel with Ring 3 supervisor-isolation fault verification."
fi

if [[ "${EXHIOS_RING3_SYSCALL_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(ring3-syscall-smoke-test)
    echo "Building the kernel with Ring 3 SYSCALL/SYSRET verification."
fi

if [[ "${EXHIOS_USER_POINTER_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(user-pointer-range-smoke-test)
    echo "Building the kernel with user-pointer range validation."
fi

if [[ "${EXHIOS_USER_COPY_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(user-copy-smoke-test)
    echo "Building the kernel with safe user-copy verification."
fi

if [[ "${EXHIOS_USER_TASK_LIFECYCLE_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(user-task-lifecycle-smoke-test)
    echo "Building the kernel with syscall-driven Ring 3 task scheduling."
fi

if [[ "${EXHIOS_DRIVER_REGISTRY_TEST:-0}" == "1" ]]; then
    KERNEL_FEATURES+=(driver-registry-smoke-test)
    echo "Building the kernel with the driver registry runtime smoke test."
fi

if [[ "${EXHIOS_VFS_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]]; then
        echo "EXHIOS_VFS_TEST requires EXHIOS_QEMU_EXIT_TEST=1." >&2
        exit 1
    fi
    KERNEL_FEATURES+=(vfs-smoke-test)
    echo "Building the kernel with the VFS/RAMFS runtime smoke test."
fi

if [[ "${EXHIOS_PROCESS_RUN_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]]; then
        echo "EXHIOS_PROCESS_RUN_TEST requires EXHIOS_QEMU_EXIT_TEST=1." >&2
        exit 1
    fi
    KERNEL_FEATURES+=(process-run-smoke-test)
    echo "Building the kernel with the first Ring 3 init execution smoke test."
elif [[ "${EXHIOS_PROCESS_LOAD_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]]; then
        echo "EXHIOS_PROCESS_LOAD_TEST requires EXHIOS_QEMU_EXIT_TEST=1." >&2
        exit 1
    fi
    KERNEL_FEATURES+=(process-load-smoke-test)
    echo "Building the kernel with the first user ELF loader runtime smoke test."
fi

if [[ "${EXHIOS_PAGING_ROLLBACK_TEST:-0}" == "1" ]]; then
    if [[ "${EXHIOS_QEMU_EXIT_TEST:-0}" != "1" ]]; then
        echo "EXHIOS_PAGING_ROLLBACK_TEST requires EXHIOS_QEMU_EXIT_TEST=1." >&2
        exit 1
    fi
    KERNEL_FEATURES+=(paging-rollback-smoke-test)
    echo "Building the kernel with the controlled page-table rollback smoke test."
fi

if (( ${#KERNEL_FEATURES[@]} == 0 )); then
    KERNEL_FEATURES+=(process-runtime)
    echo "Building the kernel with the production first-user-process runtime."
fi

FEATURE_ARGS=()
if (( ${#KERNEL_FEATURES[@]} > 0 )); then
    FEATURE_LIST="$(IFS=,; printf '%s' "${KERNEL_FEATURES[*]}")"
    FEATURE_ARGS+=(--features "$FEATURE_LIST")
fi

cargo build -p exhi-kernel --target x86_64-unknown-none "${FEATURE_ARGS[@]}"

KERNEL_ELF="$ROOT_DIR/target/x86_64-unknown-none/debug/exhi-kernel"
echo
echo "Verifying kernel ELF type..."
LC_ALL=C readelf -h "$KERNEL_ELF"
if ! LC_ALL=C readelf -h "$KERNEL_ELF" | grep -Eq 'Type:[[:space:]]+EXEC[[:space:]]+\(Executable file\)'; then
    echo "ERROR: kernel must be ET_EXEC, not PIE/ET_DYN." >&2
    exit 1
fi

echo
echo "Build completed."
echo "UEFI application: target/x86_64-unknown-uefi/debug/exhi-boot.efi"
echo "Kernel ELF:       target/x86_64-unknown-none/debug/exhi-kernel"
echo "Init ELF:         target/x86_64-unknown-none/debug/exhi-init"
