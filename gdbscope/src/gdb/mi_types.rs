/// Core data types for GDB/MI protocol output.
///
/// The GDB Machine Interface emits structured records on stdout.  Each line is
/// one of: result, async (exec / status / notify), stream (console / target /
/// log), or the prompt marker `(gdb)`.  Values inside records follow a
/// recursive grammar of c-strings, tuples, and lists.

use std::fmt;

// ---------------------------------------------------------------------------
// MiValue
// ---------------------------------------------------------------------------

/// A single value in the MI output grammar.
#[derive(Debug, Clone, PartialEq)]
pub enum MiValue {
    /// A c-string literal, e.g. `"hello"`.
    Const(String),
    /// A curly-brace tuple of key=value pairs: `{k1=v1,k2=v2}`.
    Tuple(Vec<(String, MiValue)>),
    /// A square-bracket list, which may hold plain values or key=value pairs.
    List(MiList),
}

/// The two flavours of MI list.
#[derive(Debug, Clone, PartialEq)]
pub enum MiList {
    /// `[val, val, ...]` -- homogeneous values (common for arrays of tuples).
    Values(Vec<MiValue>),
    /// `[key=val, key=val, ...]` -- named results inside a list.
    Results(Vec<(String, MiValue)>),
    /// `[]` -- empty list (could be either flavour).
    Empty,
}

impl MiValue {
    /// If this value is a `Const`, return the inner string slice.
    pub fn as_const(&self) -> Option<&str> {
        match self {
            MiValue::Const(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// If this value is a `Tuple`, return the key-value pairs.
    pub fn as_tuple(&self) -> Option<&[(String, MiValue)]> {
        match self {
            MiValue::Tuple(pairs) => Some(pairs.as_slice()),
            _ => None,
        }
    }

    /// If this value is a `List(Values(...))`, return the values slice.
    pub fn as_list_values(&self) -> Option<&[MiValue]> {
        match self {
            MiValue::List(MiList::Values(vs)) => Some(vs.as_slice()),
            MiValue::List(MiList::Empty) => Some(&[]),
            _ => None,
        }
    }

    /// Look up a key inside a `Tuple`.  Returns `None` for non-tuple values.
    pub fn get(&self, key: &str) -> Option<&MiValue> {
        match self {
            MiValue::Tuple(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Convenience: `self.get(key)?.as_const()`.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_const()
    }
}

impl fmt::Display for MiValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MiValue::Const(s) => write!(f, "\"{}\"", s),
            MiValue::Tuple(pairs) => {
                write!(f, "{{")?;
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "{}={}", k, v)?;
                }
                write!(f, "}}")
            }
            MiValue::List(list) => match list {
                MiList::Empty => write!(f, "[]"),
                MiList::Values(vs) => {
                    write!(f, "[")?;
                    for (i, v) in vs.iter().enumerate() {
                        if i > 0 {
                            write!(f, ",")?;
                        }
                        write!(f, "{}", v)?;
                    }
                    write!(f, "]")
                }
                MiList::Results(pairs) => {
                    write!(f, "[")?;
                    for (i, (k, v)) in pairs.iter().enumerate() {
                        if i > 0 {
                            write!(f, ",")?;
                        }
                        write!(f, "{}={}", k, v)?;
                    }
                    write!(f, "]")
                }
            },
        }
    }
}

// ---------------------------------------------------------------------------
// MiRecord
// ---------------------------------------------------------------------------

/// A fully parsed line of GDB/MI output.
#[derive(Debug, Clone, PartialEq)]
pub enum MiRecord {
    /// Result record: `[token]^class[,key=value]*`
    Result {
        token: Option<u64>,
        class: String,
        body: Vec<(String, MiValue)>,
    },
    /// Exec async record: `[token]*class[,key=value]*`
    AsyncExec {
        token: Option<u64>,
        class: String,
        body: Vec<(String, MiValue)>,
    },
    /// Notify async record: `=class[,key=value]*`
    AsyncNotify {
        class: String,
        body: Vec<(String, MiValue)>,
    },
    /// Status async record: `+class[,key=value]*`
    AsyncStatus {
        class: String,
        body: Vec<(String, MiValue)>,
    },
    /// Console stream output: `~"text"`
    StreamConsole(String),
    /// Target stream output: `@"text"`
    StreamTarget(String),
    /// Log stream output: `&"text"`
    StreamLog(String),
    /// The GDB prompt marker `(gdb)`.
    Prompt,
}

// ---------------------------------------------------------------------------
// MiBody trait
// ---------------------------------------------------------------------------

/// Convenience accessors for a slice of `(String, MiValue)` pairs, which is
/// the body type shared by result and async records.
pub trait MiBody {
    /// Find the first value whose key matches `key`.
    fn get(&self, key: &str) -> Option<&MiValue>;
    /// Find the first value whose key matches `key` and return it as a string.
    fn get_str(&self, key: &str) -> Option<&str>;
}

impl MiBody for [(String, MiValue)] {
    fn get(&self, key: &str) -> Option<&MiValue> {
        self.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn get_str(&self, key: &str) -> Option<&str> {
        MiBody::get(self, key)?.as_const()
    }
}
