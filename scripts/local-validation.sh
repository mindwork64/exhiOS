#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LOG_DIR="$ROOT/target/validation-logs"
mkdir -p "$LOG_DIR"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
LOG="$LOG_DIR/local-validation-$TIMESTAMP.log"

CURRENT_STEP="Инициализация"
CURRENT_COMMAND=""
START_TIME=$SECONDS

exec > >(tee "$LOG") 2>&1

report_failure() {
    local code=$?
    trap - ERR
    echo
    echo "============================================================"
    echo " VALIDATION FAILED"
    echo "============================================================"
    echo "Этап:       $CURRENT_STEP"
    echo "Команда:    ${CURRENT_COMMAND:-не определена}"
    echo "Код ошибки: $code"
    echo "Время:      $((SECONDS - START_TIME)) сек."
    echo "Полный лог: $LOG"
    echo
    echo "Диагностика (последние 50 строк):"
    tail -n 50 "$LOG" || true
    echo
    echo "Сообщения об ошибках:"
    grep -E -i '(^|[[:space:]])(error|fatal):|failed|not found|No such file|panicked|assertion' "$LOG" | tail -n 20 || true
    exit "$code"
}
trap report_failure ERR

run() {
    CURRENT_STEP="$1"
    shift
    CURRENT_COMMAND="$*"
    echo
    echo ">>> $CURRENT_STEP"
    echo "    $CURRENT_COMMAND"
    "$@"
    echo "<<< OK: $CURRENT_STEP"
}

run_qemu() {
    local name="$1"
    local test_variable="$2"

    CURRENT_STEP="QEMU: $name"
    if [[ -n "$test_variable" ]]; then
        CURRENT_COMMAND="env EXHIOS_QEMU_EXIT_TEST=1 $test_variable=1 EXHIOS_HEADLESS=1 ./scripts/run-qemu.sh"
        echo
        echo ">>> $CURRENT_STEP"
        echo "    $CURRENT_COMMAND"
        env EXHIOS_QEMU_EXIT_TEST=1 "$test_variable=1" EXHIOS_HEADLESS=1 ./scripts/run-qemu.sh
    else
        CURRENT_COMMAND="env EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_HEADLESS=1 ./scripts/run-qemu.sh"
        echo
        echo ">>> $CURRENT_STEP"
        echo "    $CURRENT_COMMAND"
        env EXHIOS_QEMU_EXIT_TEST=1 EXHIOS_HEADLESS=1 ./scripts/run-qemu.sh
    fi
    echo "<<< OK: $CURRENT_STEP"
}

section() {
    echo
    echo "============================================================"
    echo " $1"
    echo "============================================================"
}

echo "ExhiOS local validation"
echo "Repository: $ROOT"
echo "Started:    $(date)"
echo "Log:        $LOG"

section "1. Проверка зависимостей"
for tool in rustup cargo readelf qemu-system-x86_64 sgdisk mformat mmd mcopy; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        CURRENT_STEP="Проверка зависимостей"
        CURRENT_COMMAND="command -v $tool"
        echo "ОШИБКА: Не найдена обязательная программа: $tool" >&2
        false
    fi
    echo "Найдена: $tool"
done

section "2. Проверка shell-скриптов"
run "Проверка синтаксиса build.sh и run-qemu.sh" bash -n scripts/build.sh scripts/run-qemu.sh

section "3. Форматирование"
run "Проверка rustfmt" cargo fmt --all -- --check

section "4. Unit tests"
for package in \
    exhi-abi \
    exhi-elf \
    exhi-memory \
    exhi-heap \
    exhi-console \
    exhi-input \
    exhi-driver \
    exhi-task
do
    run "Тесты $package" cargo test -p "$package"
done

section "5. Rust targets"
run "Установка UEFI и kernel targets" rustup target add x86_64-unknown-uefi x86_64-unknown-none

section "6. Bootloader и kernel builds"
run "UEFI bootloader" cargo build -p exhi-boot --target x86_64-unknown-uefi
run "Kernel base build" cargo build -p exhi-kernel --target x86_64-unknown-none

section "7. Kernel feature builds"
FEATURES=(
    exception-smoke-test
    qemu-debug-exit-smoke-test
    serial-rx-smoke-test
    qemu-debug-exit-smoke-test,timer-smoke-test
    keyboard-smoke-test
    driver-registry-smoke-test
    context-switch-smoke-test
    task-accounting-smoke-test
    timer-preemption-smoke-test
    preemptive-task-exit-smoke-test
    ring3-smoke-test
    ring3-supervisor-fault-smoke-test
    ring3-syscall-smoke-test
    user-pointer-range-smoke-test
    user-copy-smoke-test
    user-task-lifecycle-smoke-test
)

for feature in "${FEATURES[@]}"; do
    run "Kernel feature: $feature" \
        cargo build -p exhi-kernel --target x86_64-unknown-none \
        --features "$feature"
done

section "8. ELF verification"
run "Проверка заголовка kernel ELF" env LC_ALL=C readelf -h target/x86_64-unknown-none/debug/exhi-kernel
CURRENT_STEP="Проверка типа kernel ELF"
CURRENT_COMMAND="readelf -h ... | grep EXEC"
echo
echo ">>> $CURRENT_STEP"
if ! LC_ALL=C readelf -h target/x86_64-unknown-none/debug/exhi-kernel | grep -Eq 'Type:[[:space:]]+EXEC[[:space:]]+\(Executable file\)'; then
    echo "ОШИБКА: kernel ELF не имеет типа EXEC (Executable file)." >&2
    false
fi
echo "<<< OK: $CURRENT_STEP"

section "9. QEMU smoke tests"
run_qemu "Base UEFI/kernel boot" ""
run_qemu "PIT IRQ0 timer" "EXHIOS_TIMER_TEST"
run_qemu "Driver registry" "EXHIOS_DRIVER_REGISTRY_TEST"
run_qemu "Cooperative context switch" "EXHIOS_CONTEXT_SWITCH_TEST"
run_qemu "Task accounting" "EXHIOS_TASK_ACCOUNTING_TEST"
run_qemu "Timer preemption" "EXHIOS_TIMER_PREEMPTION_TEST"
run_qemu "Preemptive task exit" "EXHIOS_PREEMPTIVE_TASK_EXIT_TEST"
run_qemu "Idle wakeup" "EXHIOS_IDLE_WAKEUP_TEST"
run_qemu "Ring 3 entry" "EXHIOS_RING3_TEST"
run_qemu "Ring 3 supervisor fault" "EXHIOS_RING3_SUPERVISOR_FAULT_TEST"
run_qemu "Ring 3 syscall" "EXHIOS_RING3_SYSCALL_TEST"
run_qemu "User pointer validation" "EXHIOS_USER_POINTER_TEST"
run_qemu "Safe user copy" "EXHIOS_USER_COPY_TEST"
run_qemu "Two-user-task lifecycle" "EXHIOS_USER_TASK_LIFECYCLE_TEST"

section "SUCCESS"
echo "Все локальные проверки ExhiOS завершились успешно."
echo "Время выполнения: $((SECONDS - START_TIME)) сек."
echo "Полный лог: $LOG"
