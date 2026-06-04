# gdbscope

A developer-friendly GDB TUI wrapper and reverse engineering tool. Debug C, C++, Rust, Go, and any GDB-supported language through an interactive terminal interface — no GDB commands to memorize.

Syntax-highlighted source, automatic variable inspection, memory browser with hex editing, execution trace recording with time-travel playback, cross-reference analysis, instruction patching, and hardware watchpoints.

## Quick Start

```bash
# Build
cd gdbscope && cargo build --release

# Debug a program (compile with -g for best experience)
gdbscope -e ./my_program -- arg1 arg2

# Attach to a running process
gdbscope -p 12345

# Open a core dump
gdbscope -e ./my_program -c core.1234

# Connect to a remote GDB server (QEMU, embedded, etc.)
gdbscope -r localhost:1234
```

## Screen Layout

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
| [0] Output [follow]                                                  |
|   gdb> Breakpoint 1, main() at main.c:42                            |
+----------------------------------------------------------------------+
| Timeline [REC 42 states] ...*...*....>  LIVE | step main.c:42       |
+----------------------------------------------------------------------+
| F5:Run F6:Trace F7:Into F8:Next F9:Out | ..panel hints.. | ?:Help   |
+----------------------------------------------------------------------+
```

## Panels

Press the number key shown in `[N]` in each panel title to show/hide it.

| Key | Panel | Default | Purpose |
|-----|-------|---------|---------|
| `1` | Source | on | Syntax-highlighted source with cursor, breakpoints, search |
| `2` | Stack | on | Call stack. `Enter` selects a frame (updates source + locals) |
| `3` | Locals | on | Local variables with types and values. `w`/`p`/`m` to inspect |
| `4` | Threads | off | Thread list. `Enter` switches thread context |
| `5` | Breakpoints | on | Breakpoints + watchpoints with conditions and hit counts |
| `6` | Registers | off | CPU registers (auto-loaded on every stop). `E` to edit |
| `7` | Memory | off | Hex/ASCII browser with cursor, selection, type casting, editing |
| `8` | Disasm | off | Disassembly with instruction coloring, xrefs, patching |
| `9` | Watch | off | User-defined watch expressions, re-evaluated on each stop |
| `0` | Output | on | GDB console output (commands, program output, errors) |

When no debug symbols are detected (stripped binary), the layout auto-switches to: **Disasm + Registers + Memory + Stack + Breakpoints + Output**.

## Keybindings — Complete Reference

### Execution

| Key | Action |
|-----|--------|
| `F5` | Run (if not started) or Continue (if stopped) |
| `F6` | **Trace Continue** — step line-by-line recording every state until a breakpoint is hit |
| `F7` | Step into function calls |
| `F8` | Step over (next line) |
| `F9` | Step out (finish current function) |
| `Shift+F5` | Interrupt (pause running program) |
| `Ctrl+X` | Interrupt (alternative — works on all terminals) |

### Navigation

| Key | Action |
|-----|--------|
| `j` / `k` | Move selection up / down in focused panel |
| `g` / `G` | Jump to top / bottom |
| `PgUp` / `PgDn` | Scroll by page |
| `Enter` | Activate selection (action depends on panel) |
| `Tab` / `Shift+Tab` | Cycle focus forward / backward through panels |
| `1`–`9`, `0` | Toggle panel visibility |
| `Esc` | Exit current mode / clear selection / leave panel |

### Source Panel [1]

| Key | Action |
|-----|--------|
| `j` / `k` | Move source cursor line by line |
| `Enter` | Set breakpoint at cursor line |
| `F10` | Toggle breakpoint at cursor (set or delete) |
| `.` | Jump cursor back to the current execution line |
| `/` | Search in source text |
| `n` / `N` | Next / previous search match |
| `w` | Watch — prefilled with identifier on cursor line |
| `p` | Eval — prefilled with identifier on cursor line |

### Stack Panel [2]

| Key | Action |
|-----|--------|
| `Enter` | Select frame — source, locals, and registers update to that frame's context |

### Threads Panel [4]

| Key | Action |
|-----|--------|
| `Enter` | Switch to selected thread — full context updates |

### Breakpoints Panel [5]

| Key | Action |
|-----|--------|
| `b` | Set breakpoint by location (`main`, `file.c:42`, `*0x401000`) |
| `B` | Conditional breakpoint (`main.c:42 if x > 100`) |
| `c` | Edit condition on the selected breakpoint |
| `W` | Set hardware watchpoint (`expr`, `expr r`, `expr rw`) |
| `d` | Delete selected breakpoint or watchpoint |
| `e` | Enable / disable selected breakpoint |

### Registers Panel [6]

| Key | Action |
|-----|--------|
| `E` | Edit selected register value (`rax 0x42`) |

Registers auto-load on every stop — no manual refresh needed.

### Memory Panel [7]

| Key | Action |
|-----|--------|
| `m` | Go to address (hex `0xdeadbeef` or expression `&my_var`, `buf+16`) |
| Arrow keys | Move cursor byte-by-byte (left/right) or row-by-row (up/down) |
| `PgUp` / `PgDn` | Jump by 256 bytes |
| `Enter` | Follow pointer at cursor (reads 8 bytes as address, jumps there) |
| `v` | Start / extend byte selection |
| `t` | Cycle type interpretation: hex, u8, i8, u16, u32, u64, i16, i32, i64, f32, f64, utf8 |
| `i` | Enter edit mode — type hex digits to overwrite bytes directly |
| `S` | Search memory for string or hex bytes |
| `Esc` | Exit edit mode / clear selection / leave memory panel |

**Memory search syntax:**
- String: `hello world`
- Hex bytes: `\x90\x90\x90` or `0x41 0x42 0x43`
- With start address: `0x400000 hello`

### Disassembly Panel [8]

| Key | Action |
|-----|--------|
| `j` / `k` | Move cursor through instructions |
| `Enter` | Set breakpoint at cursor address (`*0xaddr`) |
| `F10` | Toggle breakpoint at cursor address |
| `x` | Analyze cross-references (who calls here / what this calls) |
| `s` | Resolve symbol at cursor address |
| `P` | NOP out instruction at cursor (x86: fills with `0x90`) |
| `a` | Patch raw bytes at cursor address (`0xaddr hex_bytes`) |

**Instruction colors:** red = jump/branch, yellow = call, green = return, cyan = memory op, gray = nop

### Watch Panel [9]

| Key | Action |
|-----|--------|
| `w` | Add watch expression |
| `d` | Remove selected watch |
| `p` | Evaluate selected expression |
| `m` | View selected expression in memory browser |

### Output Panel [0]

| Key | Action |
|-----|--------|
| `:` | Enter raw GDB command |
| `;` | Repeat last raw command |

### Smart Inspection (global, auto-prefills from context)

| Key | Action |
|-----|--------|
| `w` | Add watch — prefilled from Source (identifier) / Locals (variable) / Watch (expression) |
| `m` | Memory — prefilled with `&variable` or pointer value from context |
| `p` | Evaluate — prefilled from focused panel context |
| `T` | Type overlay — cast memory as C struct (`0xADDR struct name`) |

### Analysis (global)

| Key | Action |
|-----|--------|
| `x` | Cross-references at disasm cursor (callers + callees) |
| `T` | Type overlay — view memory as a struct with labeled fields |
| `f` | List all known functions (output panel) |
| `s` | Resolve symbol at disasm cursor address |
| `S` | Search memory for string or hex pattern |
| `L` | Show loaded shared libraries with base addresses |

### Execution Tracing & Playback

| Key | Action |
|-----|--------|
| `F6` | Trace continue — step line-by-line to next breakpoint, recording full state each step |
| `[` / `]` | Step backward / forward through recorded history |
| `<` / `>` | Jump to previous / next breakpoint anchor |
| `{` / `}` | Jump to first recorded state / return to live |
| `R` | Toggle recording on / off |
| `C` | Clear all recorded states |
| `H` | Show value history for selected variable/register (Locals or Registers panel) |

**How tracing works:** `F6` steps the program one line at a time, recording the full debug state (stack, locals, registers, disassembly, source position) at each step, until a breakpoint is hit or the step limit is reached. You can then use `[` and `]` to walk through the trace and see exactly what happened — which variables changed, where the execution went. `<` and `>` jump between breakpoint anchors in the recording.

**Playback analysis features:**
- **Execution flow annotations** — during playback, source lines and disasm instructions show hit counts: gray=1x, yellow=2-5x, red=6+ (hot loop indicator)
- **Value history** (`H`) — shows every value change for a variable across the trace with timestamps, source locations, and trend analysis (e.g., "monotonic increase")
- **String search in memory** (`S`) — search the process memory for ASCII strings (`hello`) or hex byte patterns (`\x90\x90\x90`), with optional start address

### Patching (Disasm panel)

| Key | Action |
|-----|--------|
| `P` | NOP instruction at cursor (auto-detects instruction length) |
| `a` | Write raw bytes at address (prompted: `0xaddr hex_bytes`) |

### Input Prompts

When any prompt appears (breakpoint, watch, memory, command, search, etc.):

| Key | Action |
|-----|--------|
| `Enter` | Submit |
| `Esc` | Cancel |
| `Up` / `Down` | Browse command history |
| `Left` / `Right` | Move cursor in input |
| `Home` / `End` | Jump to start / end of input |

### General

| Key | Action |
|-----|--------|
| `?` / `F1` | Toggle help overlay (scrollable with `j`/`k`) |
| `q` | Quit (press `y` to confirm) |

## Reverse Engineering Mode

When gdbscope detects a binary without debug symbols, it automatically:
- Switches the default layout to **Disasm + Registers + Memory + Stack + Breakpoints + Output**
- Loads disassembly around the current PC on every stop
- Loads all register values on every stop
- Focuses the Disasm panel as the primary view

All features work on stripped binaries:
- Set breakpoints at addresses: `b` → `*0x401000`
- NOP out instructions: `P` in Disasm panel
- Patch bytes: `a` in Disasm panel
- Cross-references: `x` shows callers and callees
- Type overlay: `T` casts memory as structs
- Memory browser: follow pointers, search for strings, edit bytes
- Function listing: `f` shows PLT entries, dynamic symbols, and detected functions

## Syntax Highlighting

Source code is syntax-highlighted automatically based on file extension using `syntect` (~50 languages). Theme: `base16-ocean.dark`.

Supported: C, C++, Rust, Python, Go, Java, JavaScript, TypeScript, Ruby, Swift, Kotlin, Scala, Haskell, Lua, Perl, PHP, Shell, Assembly, and more.

## Configuration

```
--record-max 1000        Max recorded states (default 1000, 0 = disable)
--record-secs 300        Discard states older than N seconds (default 300, 0 = unlimited)
--trace-depth 500        Max steps per F6 trace (default 500)
--redraw-hz 30           TUI refresh rate (default 30)
--gdb-path /usr/bin/gdb  Custom GDB path
--debug                  Debug tracing to stderr
```

## Requirements

- GDB 9.0+ (for MI3 protocol)
- Terminal with 256-color or truecolor support
- Programs compiled with `-g` for source debugging (not required for RE mode)

## Building

```bash
cargo build --release
# Binary: target/release/gdbscope (~4.3 MB)
```

## Architecture

```
CLI args → Config → spawn GDB (--interpreter=mi3)
                       │
                 GdbController (async tokio task)
                   MI parser → typed records → state updates
                   command dispatch → MI command builder → GDB stdin
                   state capture → Recording buffer (ring buffer)
                   ArcSwap → lock-free snapshot publishing
                       │
                 TUI event loop (ratatui + crossterm)
                   load snapshot + recording each frame
                   render panels, handle input
                   playback mode overlays recorded state
```
