# ExhiOS userland

## Serial shell

The default kernel build starts the embedded exhi-init process in Ring 3. The initial shell uses COM1 at the kernel's configured serial settings.

Commands:

| Command | Purpose |
|---|---|
| help | Show the available commands. |
| hostname | Print the configured host name. |
| version | Print the userland version. |
| echo TEXT | Print text after the command name. |
| clear | Send the ANSI clear-screen sequence. |
| config | Print the active built-in settings. |
| exit | Terminate init. |

The line editor accepts CR, LF and CRLF, backspace and DEL. Ctrl+C clears the current line. Ctrl+D on an empty line exits. Input is bounded to 127 printable bytes; excess input is rejected with a bell.

## Configuration format

The first configuration is embedded in exhi-init:

```ini
# ExhiOS system configuration
hostname=exhios
prompt=exhios>
version=0.1
```

The format is UTF-8 key=value, with whole-line # comments, surrounding ASCII space/tab trimming and CRLF support. Parsing is allocation-free and bounded: 128 bytes per line, 32 bytes per key and 64 bytes per value. Keys are ASCII letters/digits plus _, - and .; control characters in values are rejected.

The current shell consumes hostname, prompt and version. Missing keys use built-in defaults. Duplicate known keys, malformed lines or invalid host names cause the complete configuration to fall back to defaults. Unknown syntactically valid keys are ignored for forward compatibility.

The config command shows the active values. Persistent configuration-file loading is not wired yet: P6 currently supplies the bounded in-memory VFS/RAMFS layer, while a userland file-open/read syscall and a persistent filesystem remain future work.
