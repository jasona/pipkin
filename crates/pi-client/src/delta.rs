//! Chord's Delta operations: the decoded `Op` vocabulary, the wire vocabulary with its path
//! dictionary and short forms, and an applier over `serde_json::Value`.
//!
//! One encoder/decoder pair exists per independent state stream. A decoder must observe
//! exactly the batches its matching encoder produced, beginning with that state's base.
//! Ops come from a remote process, so everything is validated: paths cannot reach the
//! prototype chain names, indexes cannot create holes, permutations must be bijections, and
//! applying an op never allocates in proportion to anything but the op's own size.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::error::{Error, Result};

fn delta(message: impl Into<String>) -> Error {
    Error::Delta(message.into())
}

/// Segments that reach the prototype chain in JavaScript consumers. Forbidden in paths (they
/// remain legal inside values).
const RESERVED_SEGMENTS: [&str; 3] = ["__proto__", "constructor", "prototype"];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Seg {
    Key(String),
    Index(usize),
}

impl Seg {
    fn to_value(&self) -> Value {
        match self {
            Seg::Key(k) => Value::String(k.clone()),
            Seg::Index(i) => Value::from(*i as u64),
        }
    }
}

pub type Path = Vec<Seg>;

pub fn parse_path(value: &Value) -> Result<Path> {
    let items = value
        .as_array()
        .ok_or_else(|| delta("path is not an array"))?;
    let mut path = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Value::String(s) => {
                if RESERVED_SEGMENTS.contains(&s.as_str()) {
                    return Err(delta(format!("unsafe path segment: {s}")));
                }
                path.push(Seg::Key(s.clone()));
            }
            Value::Number(n) => match n.as_u64() {
                Some(i) => path.push(Seg::Index(i as usize)),
                None => return Err(delta(format!("unsafe path segment: {n}"))),
            },
            other => return Err(delta(format!("unsafe path segment: {other}"))),
        }
    }
    Ok(path)
}

fn path_value(path: &Path) -> Value {
    Value::Array(path.iter().map(Seg::to_value).collect())
}

// ----------------------------------------------------------------------------- decoded ops

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// The only op that replaces a whole value.
    Replace(Value),
    Set(Path, Value),
    Delete(Path),
    /// Append text to the string at the path.
    Append(Path, String),
    /// Drop the first `n` UTF-16 code units of the string at the path.
    Trim(Path, usize),
    /// `splice(index, remove, ...items)` on the array at the path (the root when empty).
    Splice {
        path: Path,
        index: usize,
        remove: usize,
        items: Vec<Value>,
    },
    /// `new[i] = old[permutation[i]]` on the array at the path.
    Move(Path, Vec<usize>),
}

impl Op {
    /// A batch begins with a replacement exactly when it is a base.
    pub fn is_replace(&self) -> bool {
        matches!(self, Op::Replace(_))
    }

    fn validate(&self) -> Result<()> {
        match self {
            Op::Set(p, _) | Op::Delete(p) | Op::Append(p, _) | Op::Trim(p, _) if p.is_empty() => {
                Err(delta("path is empty"))
            }
            Op::Move(_, permutation) => assert_permutation(permutation),
            _ => Ok(()),
        }
    }
}

pub fn is_base(ops: &[Op]) -> bool {
    ops.first().is_some_and(Op::is_replace)
}

fn assert_permutation(permutation: &[usize]) -> Result<()> {
    let mut seen = vec![false; permutation.len()];
    for &index in permutation {
        if index >= permutation.len() || seen[index] {
            return Err(delta("m permutation is not a bijection"));
        }
        seen[index] = true;
    }
    Ok(())
}

// ----------------------------------------------------------------------------- wire ops

#[derive(Clone, Debug, PartialEq)]
pub enum PathRef {
    Inline(Path),
    Id(u64),
}

/// What crosses the wire: path interning and short forms that reuse the previous op's path.
#[derive(Clone, Debug, PartialEq)]
pub enum WireOp {
    Replace(Value),
    Set(Option<PathRef>, Value),
    Delete(Option<PathRef>),
    Append(Option<PathRef>, String),
    Trim(Option<PathRef>, usize),
    Splice {
        path: Option<PathRef>,
        index: usize,
        remove: usize,
        items: Vec<Value>,
    },
    Move(Option<PathRef>, Vec<usize>),
    /// Defines a path id; emitted on a path's second use.
    Define(u64, Path),
}

fn parse_ref(value: &Value) -> Result<PathRef> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .map(PathRef::Id)
            .ok_or_else(|| delta("bad path id")),
        Value::Array(_) => Ok(PathRef::Inline(parse_path(value)?)),
        _ => Err(delta("path is not an array")),
    }
}

fn natural(value: &Value, what: &str) -> Result<usize> {
    value
        .as_u64()
        .map(|n| n as usize)
        .ok_or_else(|| delta(format!("{what} must be a non-negative integer")))
}

fn permutation(value: &Value) -> Result<Vec<usize>> {
    let items = value
        .as_array()
        .ok_or_else(|| delta("m permutation is not an array"))?;
    let permutation = items
        .iter()
        .map(|v| natural(v, "m permutation entry"))
        .collect::<Result<Vec<_>>>()?;
    assert_permutation(&permutation)?;
    Ok(permutation)
}

/// Parse one wire op, validating its verb, arity and payload shape.
pub fn parse_wire_op(value: &Value) -> Result<WireOp> {
    let op = value
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| delta("op is not a tuple"))?;
    let verb = op[0].as_str().ok_or_else(|| delta("unknown op verb"))?;
    match (verb, op.len()) {
        ("r", 2) => Ok(WireOp::Replace(op[1].clone())),
        ("r", _) => Err(delta("r arity")),
        ("s", 3) => Ok(WireOp::Set(Some(parse_ref(&op[1])?), op[2].clone())),
        ("s", 2) => Ok(WireOp::Set(None, op[1].clone())),
        ("s", _) => Err(delta("s arity")),
        ("d", 2) => Ok(WireOp::Delete(Some(parse_ref(&op[1])?))),
        ("d", 1) => Ok(WireOp::Delete(None)),
        ("d", _) => Err(delta("d arity")),
        ("a", 3) => Ok(WireOp::Append(Some(parse_ref(&op[1])?), text(&op[2])?)),
        ("a", 2) => Ok(WireOp::Append(None, text(&op[1])?)),
        ("a", _) => Err(delta("a arity")),
        ("t", 3) => Ok(WireOp::Trim(
            Some(parse_ref(&op[1])?),
            natural(&op[2], "t count")?,
        )),
        ("t", 2) => Ok(WireOp::Trim(None, natural(&op[1], "t count")?)),
        ("t", _) => Err(delta("t arity")),
        ("p", 5) => Ok(WireOp::Splice {
            path: Some(parse_ref(&op[1])?),
            index: natural(&op[2], "p index")?,
            remove: natural(&op[3], "p remove")?,
            items: items(&op[4])?,
        }),
        ("p", 4) => Ok(WireOp::Splice {
            path: None,
            index: natural(&op[1], "p index")?,
            remove: natural(&op[2], "p remove")?,
            items: items(&op[3])?,
        }),
        ("p", _) => Err(delta("p arity")),
        ("m", 3) => Ok(WireOp::Move(Some(parse_ref(&op[1])?), permutation(&op[2])?)),
        ("m", 2) => Ok(WireOp::Move(None, permutation(&op[1])?)),
        ("m", _) => Err(delta("m arity")),
        ("#", 3) => Ok(WireOp::Define(
            natural(&op[1], "# id")? as u64,
            parse_path(&op[2])?,
        )),
        ("#", _) => Err(delta("# shape")),
        (other, _) => Err(delta(format!("unknown op verb: {other}"))),
    }
}

fn text(value: &Value) -> Result<String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| delta("a value must be a string"))
}

fn items(value: &Value) -> Result<Vec<Value>> {
    value
        .as_array()
        .cloned()
        .ok_or_else(|| delta("p items is not an array"))
}

pub fn parse_wire_ops(value: &Value) -> Result<Vec<WireOp>> {
    value
        .as_array()
        .ok_or_else(|| delta("ops is not an array"))?
        .iter()
        .map(parse_wire_op)
        .collect()
}

// ----------------------------------------------------------------------------- decoder

/// Decodes the wire batches of ONE state stream. Ids persist across batches until a base
/// batch clears them; the "same path as the previous op" short form is scoped to one batch.
#[derive(Debug, Default)]
pub struct Decoder {
    paths: HashMap<u64, Path>,
}

impl Decoder {
    pub fn new() -> Self {
        Decoder::default()
    }

    pub fn decode(&mut self, wire: &[WireOp]) -> Result<Vec<Op>> {
        let mut previous: Option<Path> = None;
        let mut out = Vec::with_capacity(wire.len());
        for op in wire {
            match op {
                WireOp::Define(id, path) => {
                    self.paths.insert(*id, path.clone());
                }
                WireOp::Replace(value) => {
                    out.push(Op::Replace(value.clone()));
                    self.paths.clear();
                    previous = None;
                }
                WireOp::Set(r, v) => {
                    let path = self.resolve(r, &mut previous, false)?;
                    out.push(Op::Set(path, v.clone()));
                }
                WireOp::Delete(r) => {
                    let path = self.resolve(r, &mut previous, false)?;
                    out.push(Op::Delete(path));
                }
                WireOp::Append(r, s) => {
                    let path = self.resolve(r, &mut previous, false)?;
                    out.push(Op::Append(path, s.clone()));
                }
                WireOp::Trim(r, n) => {
                    let path = self.resolve(r, &mut previous, false)?;
                    out.push(Op::Trim(path, *n));
                }
                WireOp::Splice {
                    path,
                    index,
                    remove,
                    items,
                } => {
                    let path = self.resolve(path, &mut previous, true)?;
                    out.push(Op::Splice {
                        path,
                        index: *index,
                        remove: *remove,
                        items: items.clone(),
                    });
                }
                WireOp::Move(r, perm) => {
                    let path = self.resolve(r, &mut previous, true)?;
                    out.push(Op::Move(path, perm.clone()));
                }
            }
        }
        Ok(out)
    }

    fn resolve(
        &self,
        r: &Option<PathRef>,
        previous: &mut Option<Path>,
        root_ok: bool,
    ) -> Result<Path> {
        let path = match r {
            None => previous
                .clone()
                .ok_or_else(|| delta("unresolvable path: []"))?,
            Some(PathRef::Inline(p)) => {
                *previous = Some(p.clone());
                p.clone()
            }
            Some(PathRef::Id(id)) => {
                let p = self
                    .paths
                    .get(id)
                    .cloned()
                    .ok_or_else(|| delta(format!("unresolvable path: {id}")))?;
                *previous = Some(p.clone());
                p
            }
        };
        if !root_ok && path.is_empty() {
            return Err(delta("unresolvable path: []"));
        }
        Ok(path)
    }
}

// ----------------------------------------------------------------------------- encoder

/// Encodes batches for ONE state stream. Used by tests and the mock server; it must stay the
/// exact inverse of `Decoder` and match Pi's encoder, which interns a path on its SECOND use.
#[derive(Debug, Default)]
pub struct Encoder {
    seen: HashSet<Path>,
    ids: HashMap<Path, u64>,
    next_id: u64,
}

impl Encoder {
    pub fn new() -> Self {
        Encoder::default()
    }

    pub fn encode(&mut self, ops: &[Op]) -> Vec<WireOp> {
        let mut previous: Option<Path> = None;
        let mut out = Vec::new();
        for op in ops {
            let path = match op {
                Op::Replace(v) => {
                    out.push(WireOp::Replace(v.clone()));
                    // A base is a recovery point: everything after it is self-contained.
                    self.seen.clear();
                    self.ids.clear();
                    self.next_id = 0;
                    previous = None;
                    continue;
                }
                Op::Set(p, _)
                | Op::Delete(p)
                | Op::Append(p, _)
                | Op::Trim(p, _)
                | Op::Move(p, _) => p,
                Op::Splice { path, .. } => path,
            };
            let reference = if previous.as_ref() == Some(path) {
                None
            } else {
                let reference = if let Some(id) = self.ids.get(path) {
                    PathRef::Id(*id)
                } else if self.seen.contains(path) {
                    let id = self.next_id;
                    self.next_id += 1;
                    self.ids.insert(path.clone(), id);
                    out.push(WireOp::Define(id, path.clone()));
                    PathRef::Id(id)
                } else {
                    self.seen.insert(path.clone());
                    PathRef::Inline(path.clone())
                };
                previous = Some(path.clone());
                Some(reference)
            };
            out.push(match op {
                Op::Set(_, v) => WireOp::Set(reference, v.clone()),
                Op::Delete(_) => WireOp::Delete(reference),
                Op::Append(_, s) => WireOp::Append(reference, s.clone()),
                Op::Trim(_, n) => WireOp::Trim(reference, *n),
                Op::Splice {
                    index,
                    remove,
                    items,
                    ..
                } => WireOp::Splice {
                    path: reference,
                    index: *index,
                    remove: *remove,
                    items: items.clone(),
                },
                Op::Move(_, perm) => WireOp::Move(reference, perm.clone()),
                Op::Replace(_) => unreachable!(),
            });
        }
        out
    }
}

/// Render a wire op as the JSON tuple that crosses the wire.
pub fn wire_op_value(op: &WireOp) -> Value {
    fn with_ref(verb: &str, r: &Option<PathRef>, rest: Vec<Value>) -> Value {
        let mut tuple = vec![Value::String(verb.into())];
        match r {
            Some(PathRef::Inline(p)) => tuple.push(path_value(p)),
            Some(PathRef::Id(id)) => tuple.push(Value::from(*id)),
            None => {}
        }
        tuple.extend(rest);
        Value::Array(tuple)
    }
    match op {
        WireOp::Replace(v) => Value::Array(vec!["r".into(), v.clone()]),
        WireOp::Set(r, v) => with_ref("s", r, vec![v.clone()]),
        WireOp::Delete(r) => with_ref("d", r, vec![]),
        WireOp::Append(r, s) => with_ref("a", r, vec![Value::String(s.clone())]),
        WireOp::Trim(r, n) => with_ref("t", r, vec![Value::from(*n as u64)]),
        WireOp::Splice {
            path,
            index,
            remove,
            items,
        } => with_ref(
            "p",
            path,
            vec![
                Value::from(*index as u64),
                Value::from(*remove as u64),
                Value::Array(items.clone()),
            ],
        ),
        WireOp::Move(r, perm) => with_ref(
            "m",
            r,
            vec![Value::Array(
                perm.iter().map(|i| Value::from(*i as u64)).collect(),
            )],
        ),
        WireOp::Define(id, path) => {
            Value::Array(vec!["#".into(), Value::from(*id), path_value(path)])
        }
    }
}

// ----------------------------------------------------------------------------- applier

fn is_container(value: &Value) -> bool {
    matches!(value, Value::Object(_) | Value::Array(_))
}

/// Walk to the container at `path`. Own properties only; an array is indexed by number only.
fn resolve_mut<'a>(root: &'a mut Value, path: &[Seg]) -> Result<&'a mut Value> {
    let mut node = root;
    for seg in path {
        node = match (node, seg) {
            (Value::Array(items), Seg::Index(i)) => {
                items.get_mut(*i).ok_or_else(|| unresolvable(path))?
            }
            (Value::Array(_), Seg::Key(k)) => {
                return Err(delta(format!("unsafe path segment: {k}")));
            }
            (Value::Object(map), Seg::Key(k)) => {
                map.get_mut(k).ok_or_else(|| unresolvable(path))?
            }
            (Value::Object(map), Seg::Index(i)) => map
                .get_mut(&i.to_string())
                .ok_or_else(|| unresolvable(path))?,
            _ => return Err(unresolvable(path)),
        };
    }
    if !is_container(node) {
        return Err(unresolvable(path));
    }
    Ok(node)
}

fn unresolvable(path: &[Seg]) -> Error {
    delta(format!("unresolvable path: {}", path_value(&path.to_vec())))
}

/// Byte offset of a UTF-16 code unit offset. A split inside a surrogate pair cannot be
/// represented by a Rust string, so it is an error rather than silent corruption.
fn utf16_to_byte_offset(text: &str, units: usize) -> Result<usize> {
    let mut seen = 0usize;
    for (byte, ch) in text.char_indices() {
        if seen == units {
            return Ok(byte);
        }
        seen += ch.len_utf16();
        if seen > units {
            return Err(delta("trim splits a surrogate pair"));
        }
    }
    Ok(text.len())
}

/// Apply `ops` to `target`, which `Replace` may discard. The target is mutated in place: on
/// error it may be partially updated, so the caller must drop the replica it came from.
pub fn apply(target: Option<Value>, ops: Vec<Op>) -> Result<Value> {
    let mut root = target.unwrap_or(Value::Null);
    for op in ops {
        op.validate()?;
        match op {
            Op::Replace(value) => root = value,
            Op::Splice {
                path,
                index,
                remove,
                items,
            } => {
                let Value::Array(array) = resolve_mut(&mut root, &path)? else {
                    return Err(unresolvable(&path));
                };
                let start = index.min(array.len());
                let end = start.saturating_add(remove).min(array.len());
                array.splice(start..end, items);
            }
            Op::Move(path, permutation) => {
                let Value::Array(array) = resolve_mut(&mut root, &path)? else {
                    return Err(unresolvable(&path));
                };
                if array.len() != permutation.len() {
                    return Err(unresolvable(&path));
                }
                let previous = std::mem::take(array);
                *array = permutation.iter().map(|&i| previous[i].clone()).collect();
            }
            Op::Set(path, value) => {
                let (parent, key) = parent_and_key(&mut root, &path)?;
                write(parent, key, value)?;
            }
            Op::Delete(path) => {
                let (parent, key) = parent_and_key(&mut root, &path)?;
                match (parent, key) {
                    (Value::Array(items), Seg::Index(i)) => {
                        if i >= items.len() {
                            return Err(unresolvable(&path));
                        }
                        items.remove(i);
                    }
                    (Value::Array(_), Seg::Key(k)) => {
                        return Err(delta(format!("unsafe path segment: {k}")));
                    }
                    (Value::Object(map), key) => {
                        map.remove(&key_name(&key));
                    }
                    _ => return Err(unresolvable(&path)),
                }
            }
            Op::Append(path, suffix) => {
                let (parent, key) = parent_and_key(&mut root, &path)?;
                let Some(Value::String(current)) = read(parent, &key) else {
                    return Err(unresolvable(&path));
                };
                let mut next = std::mem::take(current);
                next.push_str(&suffix);
                write(parent, key, Value::String(next))?;
            }
            Op::Trim(path, count) => {
                let (parent, key) = parent_and_key(&mut root, &path)?;
                let Some(Value::String(current)) = read(parent, &key) else {
                    return Err(unresolvable(&path));
                };
                let at = utf16_to_byte_offset(current, count)?;
                let next = current[at..].to_owned();
                write(parent, key, Value::String(next))?;
            }
        }
    }
    Ok(root)
}

fn key_name(seg: &Seg) -> String {
    match seg {
        Seg::Key(k) => k.clone(),
        Seg::Index(i) => i.to_string(),
    }
}

fn parent_and_key<'a>(root: &'a mut Value, path: &[Seg]) -> Result<(&'a mut Value, Seg)> {
    let (key, parent_path) = path.split_last().ok_or_else(|| delta("path is empty"))?;
    Ok((resolve_mut(root, parent_path)?, key.clone()))
}

fn read<'a>(parent: &'a mut Value, key: &Seg) -> Option<&'a mut Value> {
    match (parent, key) {
        (Value::Array(items), Seg::Index(i)) => items.get_mut(*i),
        (Value::Object(map), key) => map.get_mut(&key_name(key)),
        _ => None,
    }
}

/// Write one value. An array index may address an existing element or append exactly one past
/// the end, which keeps the value a faithful JSON array and bounds growth to the op's size.
fn write(parent: &mut Value, key: Seg, value: Value) -> Result<()> {
    match (parent, key) {
        (Value::Array(items), Seg::Index(i)) => {
            if i > items.len() {
                return Err(delta(format!("unsafe path segment: {i}")));
            }
            if i == items.len() {
                items.push(value);
            } else {
                items[i] = value;
            }
            Ok(())
        }
        (Value::Array(_), Seg::Key(k)) => Err(delta(format!("unsafe path segment: {k}"))),
        (Value::Object(map), key) => {
            map.insert(key_name(&key), value);
            Ok(())
        }
        _ => Err(delta("parent is not a container")),
    }
}

/// Build an object from entries, for tests and fixtures.
pub fn object(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect::<Map<_, _>>(),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn key(s: &str) -> Seg {
        Seg::Key(s.into())
    }

    fn path(parts: &[&str]) -> Path {
        parts.iter().map(|s| key(s)).collect()
    }

    fn wire(value: Value) -> Vec<WireOp> {
        parse_wire_ops(&value).unwrap()
    }

    #[test]
    fn applies_mixed_operations() {
        let base = json!({ "text": "a", "values": [1, 2], "nested": { "value": 1 }, "stable": { "value": 9 } });
        let ops = vec![
            Op::Append(path(&["text"]), "b".into()),
            Op::Splice {
                path: path(&["values"]),
                index: 1,
                remove: 1,
                items: vec![json!(3), json!(4)],
            },
            Op::Set(path(&["nested", "value"]), json!(2)),
        ];
        assert_eq!(
            apply(Some(base), ops).unwrap(),
            json!({ "text": "ab", "values": [1, 3, 4], "nested": { "value": 2 }, "stable": { "value": 9 } })
        );
    }

    #[test]
    fn supports_root_replacement_root_splice_and_permutation() {
        let ops = vec![Op::Replace(json!([1, 2, 3]))];
        assert!(is_base(&ops));
        let mut value = apply(None, ops).unwrap();
        value = apply(
            Some(value),
            vec![Op::Splice {
                path: vec![],
                index: 1,
                remove: 1,
                items: vec![json!(4)],
            }],
        )
        .unwrap();
        value = apply(Some(value), vec![Op::Move(vec![], vec![2, 0, 1])]).unwrap();
        assert_eq!(value, json!([3, 1, 4]));
    }

    #[test]
    fn splice_clamps_like_javascript() {
        let v = apply(
            Some(json!([1, 2, 3])),
            vec![Op::Splice {
                path: vec![],
                index: 99,
                remove: 5,
                items: vec![json!(9)],
            }],
        )
        .unwrap();
        assert_eq!(v, json!([1, 2, 3, 9]));
        let v = apply(
            Some(json!([1, 2, 3])),
            vec![Op::Splice {
                path: vec![],
                index: 1,
                remove: 99,
                items: vec![],
            }],
        )
        .unwrap();
        assert_eq!(v, json!([1]));
    }

    #[test]
    fn rejects_unsafe_and_malformed_paths() {
        for reserved in ["__proto__", "constructor", "prototype"] {
            assert!(parse_path(&json!([reserved, "x"])).is_err(), "{reserved}");
        }
        assert!(parse_path(&json!("value")).is_err());
        assert!(parse_path(&json!([1.5])).is_err());
        assert!(parse_path(&json!([-1])).is_err());
        assert!(parse_path(&json!([null])).is_err());
        // An index may append exactly one past the end, never create a hole.
        assert!(
            apply(
                Some(json!({ "values": [1] })),
                vec![Op::Set(vec![key("values"), Seg::Index(3)], json!(2))]
            )
            .is_err()
        );
        assert!(
            apply(
                Some(json!({ "values": [1] })),
                vec![Op::Set(vec![key("values"), Seg::Index(1)], json!(2))]
            )
            .is_ok()
        );
        // Appending to a non-string fails.
        assert!(
            apply(
                Some(json!({ "value": 1 })),
                vec![Op::Append(path(&["value"]), "x".into())]
            )
            .is_err()
        );
        // A permutation must be a bijection, and an empty path is only valid for p/m.
        assert!(apply(Some(json!([1, 2])), vec![Op::Move(vec![], vec![0, 0])]).is_err());
        assert!(apply(Some(json!({})), vec![Op::Set(vec![], json!(1))]).is_err());
    }

    #[test]
    fn reserved_names_are_legal_inside_values() {
        let value: Value = serde_json::from_str(r#"{"__proto__":{"z":1}}"#).unwrap();
        let out = apply(Some(json!({})), vec![Op::Set(path(&["value"]), value)]).unwrap();
        assert_eq!(out["value"]["__proto__"]["z"], 1);
    }

    #[test]
    fn deletes_array_elements_and_object_keys() {
        let v = apply(
            Some(json!({ "a": [1, 2, 3], "b": 1 })),
            vec![
                Op::Delete(vec![key("a"), Seg::Index(1)]),
                Op::Delete(path(&["b"])),
                Op::Delete(path(&["missing"])),
            ],
        )
        .unwrap();
        assert_eq!(v, json!({ "a": [1, 3] }));
        assert!(
            apply(
                Some(json!({ "a": [1] })),
                vec![Op::Delete(vec![key("a"), Seg::Index(1)])]
            )
            .is_err()
        );
    }

    #[test]
    fn trim_counts_utf16_code_units() {
        // 'a' is 1 unit, the emoji is 2 units, so trimming 3 leaves "z".
        let v = apply(
            Some(json!({ "t": "a\u{1F600}z" })),
            vec![Op::Trim(path(&["t"]), 3)],
        )
        .unwrap();
        assert_eq!(v, json!({ "t": "z" }));
        // Past the end trims everything, as JavaScript's slice does.
        let v = apply(Some(json!({ "t": "ab" })), vec![Op::Trim(path(&["t"]), 99)]).unwrap();
        assert_eq!(v, json!({ "t": "" }));
        // Splitting a surrogate pair cannot be represented and must not corrupt text.
        assert!(
            apply(
                Some(json!({ "t": "\u{1F600}z" })),
                vec![Op::Trim(path(&["t"]), 1)]
            )
            .is_err()
        );
    }

    #[test]
    fn numeric_segments_address_object_properties_by_name() {
        let v = apply(
            Some(json!({ "o": { "3": "x" } })),
            vec![Op::Set(vec![key("o"), Seg::Index(3)], json!("y"))],
        )
        .unwrap();
        assert_eq!(v, json!({ "o": { "3": "y" } }));
    }

    #[test]
    fn wire_validation_checks_arity_and_shape() {
        for bad in [
            json!([]),
            json!("r"),
            json!(["r"]),
            json!(["s"]),
            json!(["s", ["a"], 1, 2]),
            json!(["a", ["x"], 5]),
            json!(["a", 5]),
            json!(["t", -1]),
            json!(["p", 1, 2]),
            json!(["p", ["x"], 1, 1, "notarray"]),
            json!(["m", [0, 0]]),
            json!(["#", "x", ["a"]]),
            json!(["#", 0, ["__proto__"]]),
            json!(["z", 1]),
            json!(["s", ["constructor"], 1]),
            json!(["s", "value", 1, 3]),
        ] {
            assert!(parse_wire_op(&bad).is_err(), "{bad}");
        }
        assert!(parse_wire_op(&json!(["s", 1])).is_ok());
        assert!(parse_wire_op(&json!(["#", 0, ["value"]])).is_ok());
        // A bare string is not a path: unchecked it could resolve to the root.
        assert!(parse_wire_op(&json!(["s", "a", 1])).is_err());
    }

    #[test]
    fn codec_interns_paths_omits_adjacent_paths_and_round_trips() {
        let p = path(&["nested", "text"]);
        let mut enc = Encoder::new();
        let mut dec = Decoder::new();
        let first = vec![Op::Trim(p.clone(), 1), Op::Append(p.clone(), "x".into())];
        assert_eq!(dec.decode(&enc.encode(&first)).unwrap(), first);
        let second = vec![Op::Append(p.clone(), "y".into())];
        let wire_ops = enc.encode(&second);
        assert_eq!(
            wire_ops,
            wire(json!([["#", 0, ["nested", "text"]], ["a", 0, "y"]]))
        );
        assert_eq!(dec.decode(&wire_ops).unwrap(), second);
    }

    #[test]
    fn codec_resets_dictionaries_on_a_base() {
        let mut enc = Encoder::new();
        let p = path(&["value"]);
        enc.encode(&[Op::Set(p.clone(), json!(1))]);
        enc.encode(&[Op::Set(p.clone(), json!(2))]);
        assert_eq!(
            enc.encode(&[Op::Replace(json!({ "value": 3 }))]),
            wire(json!([["r", { "value": 3 }]]))
        );
        assert_eq!(
            enc.encode(&[Op::Set(p, json!(4))]),
            wire(json!([["s", ["value"], 4]]))
        );
    }

    #[test]
    fn decoder_rejects_unresolved_short_forms_and_unsafe_interned_paths() {
        assert!(Decoder::new().decode(&wire(json!([["a", "x"]]))).is_err());
        assert!(parse_wire_ops(&json!([["#", 0, ["__proto__"]], ["s", 0, true]])).is_err());
        // An unknown id is unresolvable.
        assert!(
            Decoder::new()
                .decode(&wire(json!([["a", 0, "x"]])))
                .is_err()
        );
    }

    #[test]
    fn short_form_never_spans_batches() {
        let mut enc = Encoder::new();
        let mut dec = Decoder::new();
        let p = path(&["value"]);
        let batch = enc.encode(&[Op::Set(p.clone(), json!(1)), Op::Set(p.clone(), json!(2))]);
        assert_eq!(batch, wire(json!([["s", ["value"], 1], ["s", 2]])));
        assert_eq!(dec.decode(&batch).unwrap().len(), 2);
        // A second batch starts with an explicit path (here a definition), not a short form.
        let next = enc.encode(&[Op::Set(p, json!(3))]);
        assert!(!matches!(next[0], WireOp::Set(None, _)));
    }

    #[test]
    fn interns_on_second_use_rather_than_first() {
        let mut enc = Encoder::new();
        let p = path(&["a", "deep"]);
        assert_eq!(
            enc.encode(&[Op::Append(p.clone(), "1".into())]),
            wire(json!([["a", ["a", "deep"], "1"]]))
        );
        assert_eq!(
            enc.encode(&[Op::Append(p, "2".into())]),
            wire(json!([["#", 0, ["a", "deep"]], ["a", 0, "2"]]))
        );
    }

    #[test]
    fn paths_with_null_characters_do_not_collide() {
        let ops = vec![
            Op::Set(vec![key("a\u{0}b")], json!(1)),
            Op::Set(vec![key("a"), key("b")], json!(2)),
        ];
        assert_eq!(
            Decoder::new().decode(&Encoder::new().encode(&ops)).unwrap(),
            ops
        );
    }

    #[test]
    fn decoder_ids_are_cleared_by_a_base_batch() {
        let mut dec = Decoder::new();
        dec.decode(&wire(json!([["#", 0, ["a"]], ["a", 0, "1"]])))
            .unwrap();
        dec.decode(&wire(json!([["r", { "a": "" }]]))).unwrap();
        assert!(dec.decode(&wire(json!([["a", 0, "2"]]))).is_err());
    }

    #[test]
    fn batches_after_a_base_are_self_contained() {
        let mut enc = Encoder::new();
        let p = path(&["a", "deep"]);
        enc.encode(&[Op::Append(p.clone(), "1".into())]);
        enc.encode(&[Op::Append(p.clone(), "2".into())]);
        let base = enc.encode(&[Op::Replace(json!({ "a": { "deep": "x" } }))]);
        let after = enc.encode(&[Op::Append(p.clone(), "3".into())]);
        assert_eq!(after, wire(json!([["a", ["a", "deep"], "3"]])));
        // A reader recovering from the base with a fresh decoder can follow.
        let mut dec = Decoder::new();
        assert_eq!(
            dec.decode(&base).unwrap(),
            vec![Op::Replace(json!({ "a": { "deep": "x" } }))]
        );
        assert_eq!(dec.decode(&after).unwrap(), vec![Op::Append(p, "3".into())]);
    }

    #[test]
    fn round_trips_deterministic_mixed_streams_and_converges() {
        let mut enc = Encoder::new();
        let mut dec = Decoder::new();
        let mut replica = json!({ "rows": [], "output": "", "tail": [] });
        let mut expected = replica.clone();
        for index in 0..100usize {
            let batch = vec![
                Op::Splice {
                    path: path(&["rows"]),
                    index,
                    remove: 0,
                    items: vec![json!({ "value": index })],
                },
                Op::Set(
                    vec![key("rows"), Seg::Index(index), key("value")],
                    json!(index + 1),
                ),
                Op::Append(path(&["output"]), index.to_string()),
                Op::Splice {
                    path: path(&["tail"]),
                    index,
                    remove: 0,
                    items: vec![json!(index)],
                },
            ];
            expected = apply(Some(expected), batch.clone()).unwrap();
            let decoded = dec
                .decode(
                    &parse_wire_ops(&Value::Array(
                        enc.encode(&batch).iter().map(wire_op_value).collect(),
                    ))
                    .unwrap(),
                )
                .unwrap();
            assert_eq!(decoded, batch);
            replica = apply(Some(replica), decoded).unwrap();
        }
        assert_eq!(replica, expected);
        assert_eq!(replica["rows"].as_array().unwrap().len(), 100);
    }

    #[test]
    fn wire_op_values_round_trip_through_the_parser() {
        let ops = wire(json!([
            ["r", { "a": 1 }], ["#", 0, ["x"]], ["s", 0, 1], ["s", 2], ["d"], ["a", "t"], ["t", 1],
            ["p", ["list"], 0, 1, [1]], ["p", 1, 0, []], ["m", ["list"], [1, 0]], ["m", [0]]
        ]));
        let rendered = Value::Array(ops.iter().map(wire_op_value).collect());
        assert_eq!(parse_wire_ops(&rendered).unwrap(), ops);
    }
}
