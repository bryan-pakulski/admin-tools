# gdbscope

GDB TUI wrapper for debugging and reverse engineering. C, C++, Rust, Go, anything GDB supports. No GDB commands needed.

## Usage

```
gdbscope -e ./program -- arg1 arg2     # launch
gdbscope -p 12345                      # attach to pid
gdbscope -e ./program -c core.1234     # core dump
gdbscope -r localhost:1234             # remote gdb server
```

### Options

```
-s, --source-dir PATH    Source search path (repeatable)
    --sysroot PATH       Sysroot for shared library resolution (cross-machine core dumps)
    --gdb-path PATH      GDB binary [default: gdb]
    --record-max N       Max recorded states [default: 1000, 0=disable]
    --record-secs N      Max state age in seconds [default: 300, 0=unlimited]
    --trace-depth N      Max steps per trace [default: 500]
    --redraw-hz N        TUI refresh rate [default: 30]
    --debug              Trace to stderr, disables TUI
```

### Off-site core dump analysis

When analysing core dumps from a different OS (e.g. CentOS 7 core on an OL9 dev machine), shared libraries won't match. Use `gdbscope-collect` on the production machine to bundle the core with all its libraries:

```bash
# Production machine
gdbscope-collect -e /opt/app/server -c /var/cores/core.12345
scp coredump-server-*.tar.gz devbox:~/

# Dev machine
tar xf coredump-server-*.tar.gz
gdbscope -e sysroot/opt/app/server -c sysroot/core.12345 --sysroot sysroot
```

## Layout

```
+----------------------------------------------------------------------+
| gdbscope  ./program | STOPPED | bp#1 | Thread 1 | #0 main test.c:42 |
+-----------------------------------+----------------------------------+
| [1] Source                        | [2] Stack                        |
|   => 42  int x = compute(a, b);  | [3] Locals                       |
|                                   | [5] Breakpoints                  |
+-----------------------------------+----------------------------------+
| Timeline [REC 42 states] ...*...*....>                               |
+----------------------------------------------------------------------+
| F5:Run  F7:Into  F8:Next  F9:Out                          | ?:Help   |
+----------------------------------------------------------------------+
```

## Panels

Toggle with the number key shown in the panel title.

| Key | Panel | Default | Description |
|-----|-------|---------|-------------|
| `1` | Source | on | Syntax-highlighted source with cursor, breakpoints, search |
| `2` | Stack | on | Call stack, Enter selects frame |
| `3` | Locals | on | Variables with types and values, changed values in red |
| `4` | Threads | off | Thread list, Enter switches thread |
| `5` | Breakpoints | on | Breakpoints, watchpoints, conditions, hit counts |
| `6` | Registers | off | CPU registers, changed values in red, `E` to edit |
| `7` | Memory | off | Hex/ASCII viewer with cursor, editing, type casting, pointer following |
| `8` | Disasm | off | Disassembly with call annotations, xrefs, patching |
| `9` | Watch | off | User watch expressions, re-evaluated each stop |
| `0` | Output | off | GDB console output and errors |
| `I` | Explorer | off | Interactive variable tree with drill-down |

No debug symbols detected = auto-switches to Disasm + Registers + Memory + Stack + Breakpoints.

## Keys

### Execution

| Key | Action |
|-----|--------|
| `F5` | Run / Continue |
| `F6` | Trace to next breakpoint (records full state each step) |
| `F7` | Step into |
| `F8` | Step over (auto instruction-steps when no source info) |
| `F9` | Step out |
| `Shift+F5` / `Ctrl+X` | Interrupt |

### Navigation

| Key | Action |
|-----|--------|
| `j/k` or arrows | Move selection |
| `g/G` | Top / bottom |
| `PgUp/PgDn` | Page scroll |
| `Enter` | Activate (context-dependent) |
| `Tab` / `Shift+Tab` | Cycle panel focus |
| `Esc` | Exit mode / clear / unfocus |

Mouse click focuses panels and selects items. Mouse scroll works in focused panel.

### Source

| Key | Action |
|-----|--------|
| `Enter` / `F10` | Set / toggle breakpoint |
| `.` | Jump to execution line |
| `/` | Search (`n`/`N` next/prev) |
| `w` | Watch identifier |
| `p` | Eval identifier |
| `x` | Call graph from stack |

### Disassembly

| Key | Action |
|-----|--------|
| `Enter` | Follow call/jump target |
| `.` | Jump to PC |
| `F10` | Toggle breakpoint |
| `x` | Cross-references |
| `s` | Resolve symbol |
| `P` | NOP instruction |
| `a` | Patch raw bytes |

Instruction colors: red=jump, yellow=call, green=ret, cyan=memory, gray=nop. Call targets annotated with `; -> func`.

### Breakpoints

| Key | Action |
|-----|--------|
| `b` | Set breakpoint (`main`, `file.c:42`, `*0x401000`) |
| `B` | Conditional breakpoint |
| `c` | Edit condition |
| `W` | Hardware watchpoint |
| `d` | Delete |
| `e` | Enable/disable |

### Memory

| Key | Action |
|-----|--------|
| `m` | Go to address (hex or expression like `&var`, `buf+16`) |
| `Enter` | Follow pointer |
| `v` | Start/extend selection |
| `t` | Cycle type cast (u8 u16 u32 u64 i8-i64 f32 f64 utf8) |
| `i` | Hex edit mode |
| `S` | Search memory (string or `\x90\x90` hex) |

### Registers

`E` edits selected register. Changed values shown in red.

### Watch

`w` add, `d` remove, `p` eval, `m` memory, `y` ptype. All prefilled.

### Explorer

`I` adds expression. `Enter` expands/collapses nodes. Accepts any GDB expression including casts like `*(MyType*)0x7fff5000`. C++ access specifiers flattened. Values auto-refresh on stop.

### Tracing and Playback

| Key | Action |
|-----|--------|
| `F6` | Trace to breakpoint (records frame, locals, registers, disasm per step) |
| `[` / `]` | Step backward / forward through history |
| `<` / `>` | Jump to prev / next breakpoint anchor |
| `{` / `}` | First state / return to live |
| `R` | Toggle recording |
| `C` | Clear recorded states |
| `H` | Value history for selected variable/register |

During playback, hit counts shown on source lines and disasm addresses (gray=1x, yellow=2-5x, red=6+).

### Analysis

| Key | Action |
|-----|--------|
| `x` | Cross-references (disasm) / call graph (source/stack) |
| `T` | Type overlay - cast memory as struct |
| `f` | List functions (with regex filter) |
| `s` | Resolve symbol at address |
| `S` | Search memory |
| `L` | Show loaded shared libraries |

### Smart Prefill

`w`/`p`/`m`/`y` auto-prefill from focused panel context: Source uses identifier at cursor, Locals/Watch/Explorer use selected item, Memory prefills with `&variable`.

### General

| Key | Action |
|-----|--------|
| `?` / `F1` | Help (scrollable) |
| `q` | Quit (`y` confirms) |
| `:` | Raw GDB command |
| `;` | Repeat last command |

## RE Mode

Works on stripped binaries. Auto-detected from missing debug symbols.

- `b *0x401000` for address breakpoints
- `P` / `a` for NOP / raw byte patching
- `x` for cross-references, `f` for function list
- `T` for struct overlays, `S` for memory search
- Python/Ruby/Java/Node runtime detection with stack-specific command hints

## Build

```bash
cargo build --release
# target/release/gdbscope

# Fully static binary (runs on any Linux including CentOS 7+)
make build-static
# target/static/gdbscope
```

## Requirements

- GDB 9.0+ (MI3 protocol)
- 256-color or truecolor terminal
- `-g` flag for source debugging (not needed for RE)
