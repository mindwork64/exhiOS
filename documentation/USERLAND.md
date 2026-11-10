# ExhiOS userland

## Serial shell

The production kernel starts the embedded exhi-init process in Ring 3. The initial shell uses COM1; it is not a graphical terminal. The worker is started on demand by the spawn command.

Commands:

| Command | Purpose |
|---|---|
| help | Show available commands. |
| hostname | Print the configured host name. |
| version | Print the userland version. |
| echo TEXT | Print text after the command name. |
| clear | Send the ANSI clear-screen sequence. |
| config | Print the active hostname, prompt, and version settings. |
| spawn | Start the built-in worker process. Only one worker slot is available. |
| exit | Terminate init. |

The line editor accepts CR, LF and CRLF, Backspace and DEL. Ctrl+C clears the current line. Ctrl+D on an empty line exits. Input is bounded to 127 printable bytes; excess input is rejected with a bell.

## Configuration

The initial configuration is populated by the kernel in its volatile RAMFS and read by init through the capability-checked FileRead syscall:

    hostname=exhios
    prompt=exhios>
    version=0.1.0-alpha.1

The format is UTF-8 key=value, with whole-line # comments, surrounding ASCII space/tab trimming and CRLF support. Parsing is allocation-free and bounded: 128 bytes per line, 32 bytes per key and 64 bytes per value. Keys are ASCII letters/digits plus _, - and .; control characters in values are rejected.

The current shell consumes hostname, prompt and version. Missing keys use built-in defaults. Duplicate known keys, malformed lines or invalid host names cause the complete configuration to fall back to defaults. Unknown syntactically valid keys are ignored for forward compatibility.

The config command displays the active values. RAMFS contents are kernel-populated and volatile: changing the settings at runtime or writing persistent configuration files is not part of this alpha release.
