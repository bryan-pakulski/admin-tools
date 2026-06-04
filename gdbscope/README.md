# gdbscope

GDB TUI wrapper. Debug + reverse engineer C, C++, Rust, Go, any GDB-supported language. No GDB commands needed.

Syntax-highlighted source, memory hex editor, execution trace recording with time-travel playback, cross-reference analysis, instruction patching, hardware watchpoints. Works on stripped binaries.

## Quick Start

```bash
cargo build --release

# Debug program
gdbscope -e ./my_program -- arg1 arg2

# Attach to running process
gdbscope -p 12345

# Core dump
gdbscope -e ./my_program -c core.1234

# Remote GDB server
gdbscope -r localhost:1234
```

Compile with `-g` for source debugging. Not required for RE mode.

## Layout

```
+----------------------------------------------------------------------+
| gdbscope  ./program | STOPPED | bp#1 | Thread 1 | #0 main test.c:42 |
+-----------------------------------+----------------------------------+
| [1] Source [main.c]               | [2] Stack                        |
|   => 42  int x = compute(a, b);  |   #0 compute() at math.c:15     |
|      43  printf("%d\n", x);      |   #1 main() at main.c:42        |
|                                   +----------------------------------+
|                                   | [3] Locals                       |
|                                   |   a      int    = 42             |
|                                   |   b      int    = 17             |
|                                   +----------------------------------+
|                                   | [5] Breakpoints (2)              |
|                                   |   * #1  main.c:42  main          |
|                                   |   o #2  math.c:15  if x>100     |
+-----------------------------------+----------------------------------+
| Timeline [REC 42 states] ...*...*....>  LIVE | step main.c:42       |
+----------------------------------------------------------------------+
| F5:Run F6:Trace F7:Into F8:Next F9:Out | ...hints... | ?:Help       |
+----------------------------------------------------------------------+
```

## Panels

Number key in `[N]` title toggles panel.

| Key | Panel | Default | What |
|-----|-------|---------|------|
| `1` | Source | on | Syntax-highlighted source. Cursor, breakpoints, search |
| `2` | Stack | on | Call stack. Enter selects frame |
| `3` | Locals | on | Variables + types + values. Changed values glow red |
| `4` | Threads | off | Thread list. Enter switches thread |
| `5` | Breakpoints | on | Breakpoints + watchpoints + conditions + hit counts |
| `6` | Registers | off | CPU registers. Auto-loaded. Changed values glow red. `E` edits |
| `7` | Memory | off | Hex/ASCII browser. Cursor, selection, type casting, editing, pointer following |
| `8` | Disasm | off | Disassembly. Instruction coloring, function boundaries, call target annotations, xrefs, patching |
| `9` | Watch | off | User watch expressions. Re-evaluated each stop |
| `0` | Output | off | GDB console output, errors, command results |

No debug symbols detected → auto-switches to **Disasm + Registers + Memory + Stack + Breakpoints**.

## Keybindings

### Execution

| Key | What |
|-----|------|
| `F5` | Run / Continue |
| `F6` | Trace — step-by-step to next breakpoint, records full state each hop |
| `F7` | Step into |
| `F8` | Step over. Auto instruction-steps when no source line info |
| `F9` | Step out |
| `Shift+F5` / `Ctrl+X` | Interrupt (pause) |

### Navigation

| Key | What |
|-----|------|
| `j/k` `Up/Down` | Move selection |
| `g/G` | Top / bottom |
| `PgUp/PgDn` | Page scroll |
| `Enter` | Activate (panel-specific) |
| `Tab` / `Shift+Tab` | Cycle panel focus |
| `1`-`9`, `0` | Toggle panel |
| `Esc` | Exit mode / clear / unfocus |
| Mouse click | Focus panel + select item |
| Mouse scroll | Scroll focused panel |

### Source `[1]`

| Key | What |
|-----|------|
| `Enter` | Set breakpoint at cursor line |
| `F10` | Toggle breakpoint (set/delete) |
| `.` | Jump cursor to execution line |
| `/` | Search. `n`/`N` next/prev match |
| `w` | Watch identifier (prefilled) |
| `p` | Eval identifier (prefilled) |
| `x` | Call graph from stack trace |

### Disasm `[8]`

| Key | What |
|-----|------|
| `Enter` | Follow call/jump target. Sets breakpoint on other instructions |
| `.` | Jump cursor back to PC |
| `F10` | Toggle breakpoint at address |
| `x` | Cross-references (callers + callees) |
| `s` | Resolve symbol at cursor |
| `P` | NOP instruction (x86: 0x90 fill) |
| `a` | Patch raw bytes at address |

Function boundaries shown inline. Call targets annotated with `; -> func`. Jumps show direction + offset.

Instruction colors: red=jump, yellow=call, green=ret, cyan=memory, gray=nop.

### Stack `[2]`

Enter selects frame. Source, locals, registers, disasm all update.

### Locals `[3]`

`w` watch, `p` eval, `m` memory. All prefilled from selected variable. Changed values shown in red bold.

### Threads `[4]`

Enter switches thread. Full context updates.

### Breakpoints `[5]`

| Key | What |
|-----|------|
| `b` | Set breakpoint (`main`, `file.c:42`, `*0x401000`) |
| `B` | Conditional breakpoint (`main.c:42 if x > 100`) |
| `c` | Edit condition on selected |
| `W` | Hardware watchpoint (`expr`, `expr r`, `expr rw`) |
| `d` | Delete selected |
| `e` | Enable/disable |

### Registers `[6]`

`E` edits selected register (`rax 0x42`). Changed values red bold. Auto-loaded every stop.

### Memory `[7]`

| Key | What |
|-----|------|
| `m` | Go to address (hex or expression: `&my_var`, `buf+16`) |
| Arrows | Move cursor byte-by-byte / row-by-row |
| `Enter` | Follow pointer (read 8 bytes as address, jump there) |
| `v` | Start/extend selection |
| `t` | Cycle type cast: hex u8 i8 u16 u32 u64 i16 i32 i64 f32 f64 utf8 |
| `i` | Hex edit mode — type digits to overwrite |
| `S` | Search memory (string or `\x90\x90\x90` hex) |
| `Esc` | Clear selection / exit edit / leave panel |

### Watch `[9]`

`w` add, `d` remove, `p` eval, `m` memory. All prefilled.

### Smart Inspection

`w`/`p`/`m` auto-prefill from focused panel: Source (identifier on cursor line), Locals (selected variable), Watch (selected expression). Memory prefills with `&variable` or pointer value.

### Analysis

| Key | What |
|-----|------|
| `x` | Cross-references (Disasm: instruction xrefs. Source/Stack: call graph from stack) |
| `T` | Type overlay — cast memory as C struct (`0xADDR struct name`) |
| `f` | List functions (with regex filter, max 200 shown) |
| `s` | Resolve symbol at address |
| `S` | Search memory for string/hex pattern |
| `L` | Show loaded shared libraries |

### Tracing + Playback

| Key | What |
|-----|------|
| `F6` | Trace to breakpoint. Records frame + locals + registers + disasm each step |
| `[` / `]` | Step backward / forward through history |
| `<` / `>` | Jump to prev / next breakpoint anchor |
| `{` / `}` | First recorded state / return to live |
| `R` | Toggle recording |
| `C` | Clear recorded states |
| `H` | Value history for selected variable/register (Locals/Registers panel) |

Trace captures full state each hop: frame info, local variables, registers, disassembly. Source files loaded on demand during playback.

Auto instruction-steps when no source line info (stripped binaries, runtime internals).

During playback: source lines + disasm addresses show hit counts (gray=1x, yellow=2-5x, red=6+).

### Patching

`P` NOPs instruction at cursor (auto-detects length). `a` writes raw bytes (`0x401000 eb fe`).

### Input Prompts

Enter submits. Esc cancels. Up/Down browses history. Format hints shown in footer.

### General

`?`/`F1` help (scrollable). `q` quit (y confirms). `:` raw GDB command. `;` repeat last.

## Reverse Engineering Mode

No debug symbols → layout auto-switches to Disasm + Registers + Memory + Stack + Breakpoints.

All features work on stripped binaries:
- `b *0x401000` — address breakpoint
- `P` / `a` — NOP / patch instructions
- `Enter` on call → follow target. `.` → back to PC
- `x` — xrefs. `f` — function list. `s` — symbol resolve
- `T` — struct overlay. `S` — memory search

Python/Ruby/Java/Node detected from stack frames → source panel shows runtime-specific command hints (`py-bt`, `py-list`, `py-locals`).

## Syntax Highlighting

~50 languages via `syntect`. Auto-detected from file extension. Theme: `base16-ocean.dark`.

C, C++, Rust, Python, Go, Java, JavaScript, TypeScript, Ruby, Swift, Kotlin, Scala, Haskell, Lua, Perl, PHP, Shell, Assembly, more.

## Change Highlighting

Variables + registers that changed since previous stop glow red bold. New variables in scope also highlighted. Clears on next stop.

## Config

```
--record-max 1000        Max recorded states (default 1000, 0=disable)
--record-secs 300        Max age in seconds (default 300, 0=unlimited)
--trace-depth 500        Max steps per F6 trace (default 500)
--redraw-hz 30           TUI refresh rate (default 30)
--gdb-path /usr/bin/gdb  Custom GDB path
--debug                  Debug tracing to stderr
```

## Requirements

- GDB 9.0+ (MI3 protocol)
- Terminal with 256-color or truecolor
- `-g` for source debugging (not needed for RE)

## Build

```bash
cargo build --release
# Binary: target/release/gdbscope (~4.4 MB)
```

## Architecture

```
CLI args -> Config -> spawn GDB (--interpreter=mi3)
                        |
                  GdbController (async tokio task)
                    MI parser -> typed records -> state updates
                    command dispatch -> MI command builder -> GDB stdin
                    state capture -> Recording buffer (ring buffer)
                    ArcSwap -> lock-free snapshot publishing
                        |
                  TUI event loop (ratatui + crossterm)
                    load snapshot + recording each frame
                    render panels, handle input + mouse
                    playback mode overlays recorded state
```
