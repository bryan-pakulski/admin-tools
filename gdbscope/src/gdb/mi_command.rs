/// Typed MI command builder.
///
/// Every method produces a `(token, command_string)` pair.  The token is drawn
/// from a monotonically increasing `AtomicU64` counter, which makes command ↔
/// response correlation trivial even when commands are pipelined.

use std::sync::atomic::{AtomicU64, Ordering};

/// The kind of access a hardware watchpoint monitors.
#[derive(Debug, Clone, Copy)]
pub enum WatchKind {
    /// Trigger on write (default).
    Write,
    /// Trigger on read.
    Read,
    /// Trigger on read or write.
    Access,
}

pub struct MiCommandBuilder {
    next_token: AtomicU64,
}

impl MiCommandBuilder {
    pub fn new() -> Self {
        Self {
            next_token: AtomicU64::new(1),
        }
    }

    fn next(&self) -> u64 {
        self.next_token.fetch_add(1, Ordering::Relaxed)
    }

    // -----------------------------------------------------------------
    // Execution control
    // -----------------------------------------------------------------

    /// `-exec-run [args...]`
    pub fn exec_run(&self, args: &[String]) -> (u64, String) {
        let tok = self.next();
        if args.is_empty() {
            (tok, format!("{tok}-exec-run\n"))
        } else {
            let joined = args.join(" ");
            (tok, format!("{tok}-exec-run {joined}\n"))
        }
    }

    /// `-exec-continue`
    pub fn exec_continue(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-exec-continue\n"))
    }

    /// `-exec-step` (step into)
    pub fn exec_step(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-exec-step\n"))
    }

    /// `-exec-next` (step over)
    pub fn exec_next(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-exec-next\n"))
    }

    /// `-exec-finish` (step out)
    pub fn exec_finish(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-exec-finish\n"))
    }

    /// `-exec-interrupt`
    pub fn exec_interrupt(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-exec-interrupt\n"))
    }

    // -----------------------------------------------------------------
    // Stack
    // -----------------------------------------------------------------

    /// `-stack-list-frames`
    pub fn stack_list_frames(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-stack-list-frames\n"))
    }

    /// `-stack-info-frame` — get just the current frame (faster than list-frames)
    pub fn stack_info_frame(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-stack-info-frame\n"))
    }

    /// `-stack-list-locals --all-values`
    pub fn stack_list_locals(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-stack-list-locals --all-values\n"))
    }

    /// `-stack-list-locals --simple-values` (faster — skips complex type evaluation)
    pub fn stack_list_locals_simple(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-stack-list-locals --simple-values\n"))
    }

    /// `-stack-select-frame {level}`
    pub fn stack_select_frame(&self, level: u32) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-stack-select-frame {level}\n"))
    }

    // -----------------------------------------------------------------
    // Threads
    // -----------------------------------------------------------------

    /// `-thread-info`
    pub fn thread_info(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-thread-info\n"))
    }

    /// `-thread-select {id}`
    pub fn thread_select(&self, id: i32) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-thread-select {id}\n"))
    }

    // -----------------------------------------------------------------
    // Breakpoints
    // -----------------------------------------------------------------

    /// `-break-insert {location}`
    pub fn break_insert(&self, location: &str) -> (u64, String) {
        let tok = self.next();
        let escaped = location.replace('\\', "\\\\").replace('"', "\\\"");
        (tok, format!("{tok}-break-insert \"{escaped}\"\n"))
    }

    /// `-break-delete {number}`
    pub fn break_delete(&self, number: u32) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-break-delete {number}\n"))
    }

    /// `-break-enable {number}`
    pub fn break_enable(&self, number: u32) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-break-enable {number}\n"))
    }

    /// `-break-disable {number}`
    pub fn break_disable(&self, number: u32) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-break-disable {number}\n"))
    }

    /// `-break-list`
    pub fn break_list(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-break-list\n"))
    }

    /// `-break-insert -c "condition" "location"` — insert with an inline condition.
    pub fn break_insert_cond(&self, location: &str, condition: &str) -> (u64, String) {
        let tok = self.next();
        let loc_escaped = location.replace('\\', "\\\\").replace('"', "\\\"");
        let cond_escaped = condition.replace('\\', "\\\\").replace('"', "\\\"");
        (tok, format!("{tok}-break-insert -c \"{cond_escaped}\" \"{loc_escaped}\"\n"))
    }

    /// `-break-condition {number} {expr}` — set or change a condition on an
    /// existing breakpoint.
    pub fn break_condition(&self, number: u32, expr: &str) -> (u64, String) {
        let tok = self.next();
        let escaped = expr.replace('\\', "\\\\").replace('"', "\\\"");
        (tok, format!("{tok}-break-condition {number} {escaped}\n"))
    }

    /// `-break-watch [-a|-r] "expr"` — set a hardware watchpoint.
    pub fn break_watch(&self, expr: &str, access: WatchKind) -> (u64, String) {
        let tok = self.next();
        let flag = match access {
            WatchKind::Write => "",
            WatchKind::Read => "-r ",
            WatchKind::Access => "-a ",
        };
        let escaped = expr.replace('\\', "\\\\").replace('"', "\\\"");
        (tok, format!("{tok}-break-watch {flag}\"{escaped}\"\n"))
    }

    /// Set a register value via the GDB CLI (`set $name = value`).
    ///
    /// We use `-interpreter-exec console` because the MI interface only
    /// supports setting registers by number, while the CLI form accepts
    /// symbolic names which is far more user-friendly.
    pub fn set_register(&self, name: &str, value: &str) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-interpreter-exec console \"set ${name} = {value}\"\n"))
    }

    // -----------------------------------------------------------------
    // Data
    // -----------------------------------------------------------------

    /// `-data-list-register-names`
    pub fn data_list_register_names(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-data-list-register-names\n"))
    }

    /// `-data-list-register-values {fmt}`
    ///
    /// Common formats: `"x"` for hex, `"d"` for decimal.
    pub fn data_list_register_values(&self, fmt: &str) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-data-list-register-values {fmt}\n"))
    }

    /// `-data-read-memory-bytes {addr} {count}`
    pub fn data_read_memory_bytes(&self, addr: u64, count: usize) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-data-read-memory-bytes 0x{addr:x} {count}\n"))
    }

    /// `-data-write-memory-bytes {addr} {hex_contents}`
    pub fn data_write_memory_bytes(&self, addr: u64, bytes: &[u8]) -> (u64, String) {
        let tok = self.next();
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        (tok, format!("{tok}-data-write-memory-bytes 0x{addr:x} {hex}\n"))
    }

    /// `-data-disassemble -s {start} -e {end} -- 0`
    pub fn data_disassemble_addr(&self, start: u64, end: u64) -> (u64, String) {
        let tok = self.next();
        (
            tok,
            format!("{tok}-data-disassemble -s 0x{start:x} -e 0x{end:x} -- 0\n"),
        )
    }

    /// `-data-evaluate-expression "{expr}"`
    pub fn data_evaluate_expression(&self, expr: &str) -> (u64, String) {
        let tok = self.next();
        let escaped = expr.replace('\\', "\\\\").replace('"', "\\\"");
        (
            tok,
            format!("{tok}-data-evaluate-expression \"{escaped}\"\n"),
        )
    }

    // -----------------------------------------------------------------
    // Target / file
    // -----------------------------------------------------------------

    /// `-file-exec-and-symbols {path}`
    pub fn file_exec_and_symbols(&self, path: &str) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-file-exec-and-symbols {path}\n"))
    }

    /// `-target-attach {pid}`
    pub fn target_attach(&self, pid: u32) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-target-attach {pid}\n"))
    }

    /// `-target-select remote {addr}`
    pub fn target_select_remote(&self, addr: &str) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-target-select remote {addr}\n"))
    }

    /// `-target-select core {path}`
    pub fn target_core(&self, path: &str) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-target-select core {path}\n"))
    }

    // -----------------------------------------------------------------
    // Memory search
    // -----------------------------------------------------------------

    /// Search memory for a string using GDB's `find` command.
    ///
    /// Produces: `-interpreter-exec console "find 0xSTART, +LEN, \"pattern\""`
    pub fn find_string(&self, start: u64, length: u64, pattern: &str) -> (u64, String) {
        let tok = self.next();
        let escaped = pattern
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        (
            tok,
            format!(
                "{tok}-interpreter-exec console \"find 0x{start:x}, +0x{length:x}, \\\"{escaped}\\\"\"\n"
            ),
        )
    }

    /// Search memory for a raw byte pattern using GDB's `find /b` command.
    ///
    /// Produces: `-interpreter-exec console "find /b 0xSTART, +LEN, 0xAA, 0xBB, ..."`
    pub fn find_bytes(&self, start: u64, length: u64, bytes: &[u8]) -> (u64, String) {
        let tok = self.next();
        let byte_args: String = bytes
            .iter()
            .map(|b| format!("0x{b:02x}"))
            .collect::<Vec<_>>()
            .join(", ");
        (
            tok,
            format!(
                "{tok}-interpreter-exec console \"find /b 0x{start:x}, +0x{length:x}, {byte_args}\"\n"
            ),
        )
    }

    // -----------------------------------------------------------------
    // Analysis commands (xrefs, type overlay, function listing)
    // -----------------------------------------------------------------

    /// `info functions [regexp]` -- list matching functions via CLI.
    pub fn info_functions(&self, pattern: &str) -> (u64, String) {
        let tok = self.next();
        let escaped = pattern.replace('\\', "\\\\").replace('"', "\\\"");
        (tok, format!("{tok}-interpreter-exec console \"info functions {escaped}\"\n"))
    }

    /// `-symbol-info-functions [--name regexp]` — structured function listing.
    pub fn symbol_info_functions(&self, pattern: Option<&str>) -> (u64, String) {
        let tok = self.next();
        match pattern {
            Some(p) => {
                let escaped = p.replace('\\', "\\\\").replace('"', "\\\"");
                (tok, format!("{tok}-symbol-info-functions --name \"{escaped}\"\n"))
            }
            None => (tok, format!("{tok}-symbol-info-functions\n")),
        }
    }

    /// `disassemble func` -- disassemble a named function to find call targets.
    pub fn disassemble_function(&self, func: &str) -> (u64, String) {
        let tok = self.next();
        let escaped = func.replace('\\', "\\\\").replace('"', "\\\"");
        (tok, format!("{tok}-interpreter-exec console \"disassemble {escaped}\"\n"))
    }

    /// `ptype type_name` -- show type structure via CLI.
    pub fn ptype(&self, type_name: &str) -> (u64, String) {
        let tok = self.next();
        let escaped = type_name.replace('\\', "\\\\").replace('"', "\\\"");
        (tok, format!("{tok}-interpreter-exec console \"ptype {escaped}\"\n"))
    }

    /// Print memory as a typed value: `*(type*)addr` via `-data-evaluate-expression`.
    pub fn print_typed(&self, type_expr: &str, addr: u64) -> (u64, String) {
        let tok = self.next();
        let escaped = type_expr.replace('\\', "\\\\").replace('"', "\\\"");
        (tok, format!("{tok}-data-evaluate-expression \"*({escaped}*)0x{addr:x}\"\n"))
    }

    /// `info symbol addr` -- resolve an address to the nearest symbol.
    pub fn info_symbol(&self, addr: u64) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-interpreter-exec console \"info symbol 0x{addr:x}\"\n"))
    }

    // -----------------------------------------------------------------
    // Raw CLI command via MI
    // -----------------------------------------------------------------

    /// `-interpreter-exec console "{cmd}"`
    ///
    /// Passes an arbitrary GDB CLI command through the MI interface.
    /// Quotes inside `cmd` are escaped.
    pub fn cli_command(&self, cmd: &str) -> (u64, String) {
        let tok = self.next();
        let escaped = cmd.replace('\\', "\\\\").replace('"', "\\\"");
        (
            tok,
            format!("{tok}-interpreter-exec console \"{escaped}\"\n"),
        )
    }

    pub fn gdb_exit(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-gdb-exit\n"))
    }

    pub fn target_detach(&self) -> (u64, String) {
        let tok = self.next();
        (tok, format!("{tok}-target-detach\n"))
    }
}

impl Default for MiCommandBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_monotonic() {
        let b = MiCommandBuilder::new();
        let (t1, _) = b.exec_continue();
        let (t2, _) = b.exec_step();
        assert!(t2 > t1);
    }

    #[test]
    fn exec_run_no_args() {
        let b = MiCommandBuilder::new();
        let (tok, cmd) = b.exec_run(&[]);
        assert_eq!(cmd, format!("{tok}-exec-run\n"));
    }

    #[test]
    fn exec_run_with_args() {
        let b = MiCommandBuilder::new();
        let args = vec!["--flag".to_string(), "value".to_string()];
        let (tok, cmd) = b.exec_run(&args);
        assert_eq!(cmd, format!("{tok}-exec-run --flag value\n"));
    }

    #[test]
    fn stack_list_locals_has_all_values() {
        let b = MiCommandBuilder::new();
        let (tok, cmd) = b.stack_list_locals();
        assert_eq!(cmd, format!("{tok}-stack-list-locals --all-values\n"));
    }

    #[test]
    fn data_disassemble_format() {
        let b = MiCommandBuilder::new();
        let (tok, cmd) = b.data_disassemble_addr(0x1000, 0x2000);
        assert_eq!(
            cmd,
            format!("{tok}-data-disassemble -s 0x1000 -e 0x2000 -- 0\n")
        );
    }

    #[test]
    fn target_core_format() {
        let b = MiCommandBuilder::new();
        let (tok, cmd) = b.target_core("/tmp/core.1234");
        assert_eq!(cmd, format!("{tok}-target-select core /tmp/core.1234\n"));
    }

    #[test]
    fn cli_command_escapes_quotes() {
        let b = MiCommandBuilder::new();
        let (tok, cmd) = b.cli_command("print \"hello\"");
        assert_eq!(
            cmd,
            format!("{tok}-interpreter-exec console \"print \\\"hello\\\"\"\n")
        );
    }

    #[test]
    fn break_insert_format() {
        let b = MiCommandBuilder::new();
        let (tok, cmd) = b.break_insert("main.c:42");
        assert_eq!(cmd, format!("{tok}-break-insert \"main.c:42\"\n"));
    }

    #[test]
    fn data_read_memory_bytes_format() {
        let b = MiCommandBuilder::new();
        let (tok, cmd) = b.data_read_memory_bytes(0xdead_beef, 256);
        assert_eq!(
            cmd,
            format!("{tok}-data-read-memory-bytes 0xdeadbeef 256\n")
        );
    }

    #[test]
    fn data_evaluate_expression_escapes() {
        let b = MiCommandBuilder::new();
        let (tok, cmd) = b.data_evaluate_expression("arr[0]");
        assert_eq!(
            cmd,
            format!("{tok}-data-evaluate-expression \"arr[0]\"\n")
        );
    }

    #[test]
    fn thread_select_format() {
        let b = MiCommandBuilder::new();
        let (tok, cmd) = b.thread_select(3);
        assert_eq!(cmd, format!("{tok}-thread-select 3\n"));
    }
}
