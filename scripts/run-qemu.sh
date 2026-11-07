#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
cd "$ROOT_DIR"

EXCEPTION_TEST="${EXHIOS_EXCEPTION_TEST:-0}"
QEMU_EXIT_TEST="${EXHIOS_QEMU_EXIT_TEST:-0}"
HEADLESS_TEST="${EXHIOS_HEADLESS:-0}"
TIMER_TEST="${EXHIOS_TIMER_TEST:-0}"
KEYBOARD_TEST="${EXHIOS_KEYBOARD_TEST:-0}"
DRIVER_REGISTRY_TEST="${EXHIOS_DRIVER_REGISTRY_TEST:-0}"
TASK_ACCOUNTING_TEST="${EXHIOS_TASK_ACCOUNTING_TEST:-0}"
TIMER_PREEMPTION_TEST="${EXHIOS_TIMER_PREEMPTION_TEST:-0}"
CONTEXT_SWITCH_TEST="${EXHIOS_CONTEXT_SWITCH_TEST:-0}"
SERIAL_RX_TEST="${EXHIOS_SERIAL_RX_TEST:-0}"

if [[ "$EXCEPTION_TEST" == "1" && "$QEMU_EXIT_TEST" == "1" ]]; then
    echo "Choose either EXHIOS_EXCEPTION_TEST or EXHIOS_QEMU_EXIT_TEST, not both." >&2
    exit 1
fi

if [[ "$TIMER_TEST" == "1" && "$QEMU_EXIT_TEST" != "1" ]]; then
    echo "EXHIOS_TIMER_TEST requires EXHIOS_QEMU_EXIT_TEST=1 for controlled QEMU shutdown." >&2
    exit 1
fi
if [[ "$TIMER_TEST" == "1" && "$EXCEPTION_TEST" == "1" ]]; then
    echo "EXHIOS_TIMER_TEST cannot be combined with EXHIOS_EXCEPTION_TEST." >&2
    exit 1
fi

if [[ "$KEYBOARD_TEST" == "1" && "$QEMU_EXIT_TEST" != "1" ]]; then
    echo "EXHIOS_KEYBOARD_TEST requires EXHIOS_QEMU_EXIT_TEST=1 for controlled QEMU shutdown." >&2
    exit 1
fi
if [[ "$KEYBOARD_TEST" == "1" && ( "$EXCEPTION_TEST" == "1" || "$SERIAL_RX_TEST" == "1" || "$TIMER_TEST" == "1" ) ]]; then
    echo "EXHIOS_KEYBOARD_TEST cannot be combined with exception, serial RX or timer tests." >&2
    exit 1
fi

if [[ "$CONTEXT_SWITCH_TEST" == "1" && "$QEMU_EXIT_TEST" != "1" ]]; then
    echo "EXHIOS_CONTEXT_SWITCH_TEST requires EXHIOS_QEMU_EXIT_TEST=1 for controlled QEMU shutdown." >&2
    exit 1
fi
if [[ "$CONTEXT_SWITCH_TEST" == "1" && ( "$EXCEPTION_TEST" == "1" || "$SERIAL_RX_TEST" == "1" || "$TIMER_TEST" == "1" || "$KEYBOARD_TEST" == "1" || "$DRIVER_REGISTRY_TEST" == "1" ) ]]; then
    echo "EXHIOS_CONTEXT_SWITCH_TEST cannot be combined with other smoke-test modes." >&2
    exit 1
fi

if [[ "$TASK_ACCOUNTING_TEST" == "1" && "$QEMU_EXIT_TEST" != "1" ]]; then
    echo "EXHIOS_TASK_ACCOUNTING_TEST requires EXHIOS_QEMU_EXIT_TEST=1 for controlled QEMU shutdown." >&2
    exit 1
fi
if [[ "$TASK_ACCOUNTING_TEST" == "1" && ( "$EXCEPTION_TEST" == "1" || "$SERIAL_RX_TEST" == "1" || "$TIMER_TEST" == "1" || "$KEYBOARD_TEST" == "1" || "$CONTEXT_SWITCH_TEST" == "1" || "$DRIVER_REGISTRY_TEST" == "1" ) ]]; then
    echo "EXHIOS_TASK_ACCOUNTING_TEST cannot be combined with other smoke-test modes." >&2
    exit 1
fi

if [[ "$TIMER_PREEMPTION_TEST" == "1" && "$QEMU_EXIT_TEST" != "1" ]]; then
    echo "EXHIOS_TIMER_PREEMPTION_TEST requires EXHIOS_QEMU_EXIT_TEST=1 for controlled QEMU shutdown." >&2
    exit 1
fi
if [[ "$TIMER_PREEMPTION_TEST" == "1" && ( "$EXCEPTION_TEST" == "1" || "$SERIAL_RX_TEST" == "1" || "$TIMER_TEST" == "1" || "$KEYBOARD_TEST" == "1" || "$CONTEXT_SWITCH_TEST" == "1" || "$TASK_ACCOUNTING_TEST" == "1" || "$DRIVER_REGISTRY_TEST" == "1" ) ]]; then
    echo "EXHIOS_TIMER_PREEMPTION_TEST cannot be combined with other smoke-test modes." >&2
    exit 1
fi

if [[ "$DRIVER_REGISTRY_TEST" == "1" && "$QEMU_EXIT_TEST" != "1" ]]; then
    echo "EXHIOS_DRIVER_REGISTRY_TEST requires EXHIOS_QEMU_EXIT_TEST=1 for controlled QEMU shutdown." >&2
    exit 1
fi
if [[ "$DRIVER_REGISTRY_TEST" == "1" && ( "$EXCEPTION_TEST" == "1" || "$SERIAL_RX_TEST" == "1" || "$TIMER_TEST" == "1" || "$KEYBOARD_TEST" == "1" ) ]]; then
    echo "EXHIOS_DRIVER_REGISTRY_TEST cannot be combined with other smoke-test modes." >&2
    exit 1
fi

"$ROOT_DIR/scripts/build.sh"

for tool in qemu-system-x86_64 sgdisk mformat mmd mcopy; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Missing required tool: $tool" >&2
        echo "On Debian/Ubuntu install: qemu-system-x86 ovmf gdisk mtools dosfstools" >&2
        exit 1
    fi
done

find_file() {
    for candidate in "$@"; do
        if [[ -n "$candidate" && -f "$candidate" ]]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    return 1
}

OVMF_CODE_PATH="$(find_file \
    "${OVMF_CODE:-}" \
    /usr/share/OVMF/OVMF_CODE.fd \
    /usr/share/OVMF/OVMF_CODE_4M.fd \
    /usr/share/edk2/x64/OVMF_CODE.fd \
    /usr/share/qemu/OVMF_CODE.fd)" || {
        echo "Could not find OVMF_CODE.fd. Set OVMF_CODE to its path." >&2
        exit 1
    }

OVMF_VARS_PATH="$(find_file \
    "${OVMF_VARS:-}" \
    /usr/share/OVMF/OVMF_VARS.fd \
    /usr/share/OVMF/OVMF_VARS_4M.fd \
    /usr/share/edk2/x64/OVMF_VARS.fd \
    /usr/share/qemu/OVMF_VARS.fd)" || {
        echo "Could not find OVMF_VARS.fd. Set OVMF_VARS to its path." >&2
        exit 1
    }

ESP_IMAGE="$ROOT_DIR/target/exhios-esp.img"
VARS_COPY="$ROOT_DIR/target/exhios-ovmf-vars.fd"
ESP_OFFSET=1048576

mkdir -p "$ROOT_DIR/target"
rm -f "$ESP_IMAGE"
truncate -s 128M "$ESP_IMAGE"

sgdisk --clear \
    --new=1:2048:0 \
    --typecode=1:EF00 \
    --change-name=1:EFI-System \
    "$ESP_IMAGE"

ESP_MTOOLS_IMAGE="${ESP_IMAGE}@@${ESP_OFFSET}"
mformat -i "$ESP_MTOOLS_IMAGE" -F ::
mmd -i "$ESP_MTOOLS_IMAGE" ::/EFI
mmd -i "$ESP_MTOOLS_IMAGE" ::/EFI/BOOT
mcopy -i "$ESP_MTOOLS_IMAGE" \
    "$ROOT_DIR/target/x86_64-unknown-uefi/debug/exhi-boot.efi" \
    ::/EFI/BOOT/BOOTX64.EFI
mcopy -i "$ESP_MTOOLS_IMAGE" \
    "$ROOT_DIR/target/x86_64-unknown-none/debug/exhi-kernel" \
    ::/exhi-kernel.elf

cp "$OVMF_VARS_PATH" "$VARS_COPY"

QEMU_ARGS=(
    -machine q35
    -m 512M
    -drive "if=pflash,format=raw,readonly=on,file=$OVMF_CODE_PATH"
    -drive "if=pflash,format=raw,file=$VARS_COPY"
    -drive "if=none,id=esp,format=raw,file=$ESP_IMAGE"
    -device qemu-xhci,id=xhci
    -device usb-storage,bus=xhci.0,drive=esp,bootindex=1
    -net none
    -serial stdio
    -no-reboot
)

if [[ "$HEADLESS_TEST" == "1" ]]; then
    QEMU_ARGS+=(-display none -monitor none)
fi

if [[ "$TIMER_PREEMPTION_TEST" == "1" ]]; then
    echo "Starting ExhiOS PIT timer-driven task preemption runtime smoke test."
    QEMU_ARGS+=(-device isa-debug-exit,iobase=0xf4,iosize=0x04)
elif [[ "$CONTEXT_SWITCH_TEST" == "1" ]]; then
    echo "Starting ExhiOS x86-64 cooperative context-switch runtime smoke test."
    QEMU_ARGS+=(-device isa-debug-exit,iobase=0xf4,iosize=0x04)
elif [[ "$TASK_ACCOUNTING_TEST" == "1" ]]; then
    echo "Starting ExhiOS PIT-backed cooperative task runtime-accounting smoke test."
    QEMU_ARGS+=(-device isa-debug-exit,iobase=0xf4,iosize=0x04)
elif [[ "$DRIVER_REGISTRY_TEST" == "1" ]]; then
    echo "Starting ExhiOS driver registry runtime smoke test."
    QEMU_ARGS+=(-device isa-debug-exit,iobase=0xf4,iosize=0x04)
elif [[ "$KEYBOARD_TEST" == "1" ]]; then
    echo "Starting ExhiOS interactive PS/2 keyboard IRQ1 smoke test."
    QEMU_ARGS+=(-device isa-debug-exit,iobase=0xf4,iosize=0x04)
elif [[ "$TIMER_TEST" == "1" ]]; then
    echo "Starting ExhiOS PIT IRQ0 timer smoke test."
    QEMU_ARGS+=(-device isa-debug-exit,iobase=0xf4,iosize=0x04)
elif [[ "$QEMU_EXIT_TEST" == "1" ]]; then
    echo "Starting ExhiOS QEMU debug-exit test."
    # isa-debug-exit requests a guest shutdown; -no-shutdown would keep QEMU open.
    QEMU_ARGS+=(-device isa-debug-exit,iobase=0xf4,iosize=0x04)
elif [[ "$EXCEPTION_TEST" == "1" ]]; then
    echo "Starting ExhiOS IDT/INT3 exception smoke test in QEMU."
else
    echo "Starting ExhiOS kernel foundation test in QEMU."
fi

if [[ "$QEMU_EXIT_TEST" != "1" ]]; then
    QEMU_ARGS+=(-no-shutdown)
fi

echo "EFI application and kernel ELF are on the same GPT EFI System Partition."

set +e
qemu-system-x86_64 "${QEMU_ARGS[@]}"
QEMU_STATUS=$?
set -e

if [[ "$QEMU_EXIT_TEST" == "1" ]]; then
    if [[ "$QEMU_STATUS" -eq 33 ]]; then
        echo "QEMU debug-exit confirmed (exit status 33)."
        exit 0
    fi
    echo "ERROR: expected QEMU debug-exit status 33, received $QEMU_STATUS." >&2
    exit 1
fi

exit "$QEMU_STATUS"
