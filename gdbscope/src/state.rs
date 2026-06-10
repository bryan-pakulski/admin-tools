use std::sync::Arc;

use arc_swap::ArcSwap;

/// Maximum number of output lines retained in the ring buffer.
pub const OUTPUT_CAP: usize = 10_000;

// ---------------------------------------------------------------------------
// Target lifecycle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetState {
    NotStarted,
    Running,
    Stopped,
    Exited(i32),
    Terminated,
}

#[derive(Debug, Clone)]
pub enum StopReason {
    BreakpointHit { number: u32 },
    Watchpoint { number: u32 },
    StepFinished,
    SignalReceived { name: String, meaning: String },
    FunctionFinished,
    ExitedNormally { code: i32 },
    Unknown(String),
}

// ---------------------------------------------------------------------------
// Stack / frames
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Frame {
    pub level: u32,
    pub addr: u64,
    pub func: Option<String>,
    pub file: Option<String>,
    pub fullname: Option<String>,
    pub line: Option<u32>,
    pub args: Vec<FuncArg>,
}

#[derive(Debug, Clone)]
pub struct FuncArg {
    pub name: String,
    pub value: String,
}

// ---------------------------------------------------------------------------
// Threads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Thread {
    pub id: i32,
    pub target_id: String,
    pub name: Option<String>,
    pub state: String,
    pub frame: Option<Frame>,
}

// ---------------------------------------------------------------------------
// Variables / locals
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub type_name: String,
}

// ---------------------------------------------------------------------------
// Breakpoints
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Breakpoint {
    pub number: u32,
    pub enabled: bool,
    pub bp_type: String,
    pub address: Option<u64>,
    pub func: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub hit_count: u32,
    pub original_location: String,
    pub condition: Option<String>,
}

// ---------------------------------------------------------------------------
// Registers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Register {
    pub number: u32,
    pub name: String,
    pub value: String,
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MemoryBlock {
    pub address: u64,
    pub bytes: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Disassembly
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DisasmInstruction {
    pub address: u64,
    pub func_name: Option<String>,
    pub offset: Option<u32>,
    pub inst: String,
}

// ---------------------------------------------------------------------------
// Cross-references (xrefs)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XrefType {
    /// Something calls the target address.
    CallTo,
    /// The target address calls this address.
    CallFrom,
    /// A jump to the target address.
    JumpTo,
}

#[derive(Debug, Clone)]
pub struct XrefEntry {
    pub address: u64,
    pub func_name: Option<String>,
    pub xref_type: XrefType,
    /// The instruction text at the xref site.
    pub context: String,
}

// ---------------------------------------------------------------------------
// Type overlay
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TypeOverlayField {
    pub name: String,
    pub type_name: String,
    pub offset: usize,
    pub size: usize,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct TypeOverlay {
    pub type_name: String,
    pub address: u64,
    pub total_size: usize,
    pub fields: Vec<TypeOverlayField>,
}

// ---------------------------------------------------------------------------
// Explorer (interactive object inspector)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ExplorerNode {
    pub var_name: String,
    pub display_name: String,
    pub type_name: String,
    pub value: String,
    pub has_children: bool,
    pub expanded: bool,
    pub children_loaded: bool,
    pub depth: u16,
    pub is_root: bool,
    pub changed: bool,
}

// ---------------------------------------------------------------------------
// Mapped libraries / shared objects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MappedLibrary {
    pub name: String,           // library id (path)
    pub target_name: String,    // target-visible name
    pub base_addr: Option<u64>, // base load address from ranges
    pub symbols_loaded: bool,
}

// ---------------------------------------------------------------------------
// Watch expressions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct WatchExpression {
    pub id: u32,
    pub expression: String,
    pub value: String,
    pub type_name: String,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct SyntaxColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone)]
pub struct StyledSegment {
    pub text: String,
    pub fg: SyntaxColor,
}

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: String,
    pub lines: Vec<String>,
    pub highlighted: Vec<Vec<StyledSegment>>,
}

// ---------------------------------------------------------------------------
// Console output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputKind {
    Console,
    Target,
    Log,
    Error,
    Info,
}

#[derive(Debug, Clone)]
pub struct GdbOutputLine {
    pub kind: OutputKind,
    pub text: String,
}

// ---------------------------------------------------------------------------
// Master snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GdbSnapshot {
    pub target_state: TargetState,
    pub stop_reason: Option<StopReason>,

    // Threads
    pub threads: Vec<Thread>,
    pub current_thread_id: Option<i32>,

    // Stack
    pub stack: Vec<Frame>,
    pub current_frame_level: u32,

    // Locals for the current frame
    pub locals: Vec<Variable>,

    // Breakpoints
    pub breakpoints: Vec<Breakpoint>,

    // Registers
    pub registers: Vec<Register>,
    pub register_names: Vec<String>,

    // Memory viewer
    pub memory: Option<MemoryBlock>,
    pub memory_address: u64,

    // Disassembly
    pub disasm: Vec<DisasmInstruction>,

    // Cross-references
    pub xrefs: Vec<XrefEntry>,

    // Type overlay
    pub type_overlay: Option<TypeOverlay>,

    // Watch expressions
    pub watch_expressions: Vec<WatchExpression>,

    // Explorer (interactive type inspector tree)
    pub explorer_nodes: Vec<ExplorerNode>,

    // Mapped libraries / shared objects
    pub mapped_libs: Vec<MappedLibrary>,

    // Source viewer
    pub source: Option<SourceFile>,
    pub source_line: Option<u32>,
    pub source_loading: bool,

    // Console output (capped ring buffer)
    pub output: Vec<GdbOutputLine>,

    // Status bar
    pub status_message: Option<String>,
    pub target_executable: Option<String>,

    // Recording
    pub recording_count: usize,

    // Debug info availability (false for stripped / no-symbols binaries)
    pub has_debug_info: bool,
}

impl GdbSnapshot {
    /// Create an empty default snapshot with no target loaded.
    pub fn empty() -> Self {
        Self {
            target_state: TargetState::NotStarted,
            stop_reason: None,
            threads: Vec::new(),
            current_thread_id: None,
            stack: Vec::new(),
            current_frame_level: 0,
            locals: Vec::new(),
            breakpoints: Vec::new(),
            registers: Vec::new(),
            register_names: Vec::new(),
            memory: None,
            memory_address: 0,
            disasm: Vec::new(),
            xrefs: Vec::new(),
            type_overlay: None,
            watch_expressions: Vec::new(),
            explorer_nodes: Vec::new(),
            mapped_libs: Vec::new(),
            source: None,
            source_line: None,
            source_loading: false,
            output: Vec::new(),
            status_message: None,
            target_executable: None,
            recording_count: 0,
            has_debug_info: false,
        }
    }

    /// Append an output line to the console buffer, evicting the oldest entries
    /// when the buffer exceeds [`OUTPUT_CAP`].
    pub fn push_output(&mut self, kind: OutputKind, text: String) {
        self.output.push(GdbOutputLine { kind, text });
        if self.output.len() > OUTPUT_CAP {
            let excess = self.output.len() - OUTPUT_CAP;
            self.output.drain(..excess);
        }
    }
}

// ---------------------------------------------------------------------------
// Shared state handle (ArcSwap)
// ---------------------------------------------------------------------------

/// Thread-safe, lock-free shared state.
///
/// Writers clone the current snapshot via `load()`, mutate it, then publish
/// with `store()`.  Readers always get a consistent, immutable view.
pub type SharedState = Arc<ArcSwap<GdbSnapshot>>;

/// Create a new [`SharedState`] initialised to [`GdbSnapshot::empty()`].
pub fn new_shared() -> SharedState {
    Arc::new(ArcSwap::from_pointee(GdbSnapshot::empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_snapshot_defaults() {
        let snap = GdbSnapshot::empty();
        assert_eq!(snap.target_state, TargetState::NotStarted);
        assert!(snap.threads.is_empty());
        assert!(snap.output.is_empty());
        assert!(snap.stop_reason.is_none());
    }

    #[test]
    fn push_output_caps_at_limit() {
        let mut snap = GdbSnapshot::empty();
        for i in 0..OUTPUT_CAP + 500 {
            snap.push_output(OutputKind::Console, format!("line {i}"));
        }
        assert_eq!(snap.output.len(), OUTPUT_CAP);
        // The oldest 500 lines should have been evicted; the first remaining
        // line should be "line 500".
        assert_eq!(snap.output[0].text, "line 500");
    }

    #[test]
    fn shared_state_load_store() {
        let shared = new_shared();
        let mut snap = (**shared.load()).clone();
        snap.status_message = Some("hello".into());
        shared.store(Arc::new(snap));
        let loaded = shared.load();
        assert_eq!(loaded.status_message.as_deref(), Some("hello"));
    }
}
