//! Unified diff construction for the fixture file changes.

use desktop_core::{DiffKind, DiffLine, FileChange, Hunk};

const CONTEXT: usize = 3;

#[derive(Clone, Copy, PartialEq)]
enum Op {
    Equal,
    Remove,
    Add,
}

/// Longest-common-subsequence line diff. Quadratic, which is fine for fixture-sized files.
fn line_ops(old: &[&str], new: &[&str]) -> Vec<(Op, usize, usize)> {
    let (n, m) = (old.len(), new.len());
    let width = m + 1;
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * width + j] = if old[i] == new[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut ops = Vec::with_capacity(n + m);
    while i < n || j < m {
        if i < n && j < m && old[i] == new[j] {
            ops.push((Op::Equal, i, j));
            i += 1;
            j += 1;
        } else if j < m && (i == n || table[i * width + j + 1] >= table[(i + 1) * width + j]) {
            ops.push((Op::Add, i, j));
            j += 1;
        } else {
            ops.push((Op::Remove, i, j));
            i += 1;
        }
    }
    ops
}

/// Build a `FileChange` with unified hunks (three lines of context) from two file bodies.
pub fn unified_diff(path: &str, old: &str, new: &str) -> FileChange {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let ops = line_ops(&old, &new);

    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, o)| o.0 != Op::Equal)
        .map(|(i, _)| i)
        .collect();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &c in &changed {
        let start = c.saturating_sub(CONTEXT);
        let end = (c + CONTEXT + 1).min(ops.len());
        match ranges.last_mut() {
            Some(last) if start <= last.1 => last.1 = end,
            _ => ranges.push((start, end)),
        }
    }

    let mut added = 0;
    let mut removed = 0;
    let mut hunks = Vec::new();
    for (start, end) in ranges {
        let mut lines = Vec::new();
        let (mut old_count, mut new_count) = (0u32, 0u32);
        for &(op, i, j) in &ops[start..end] {
            match op {
                Op::Equal => {
                    old_count += 1;
                    new_count += 1;
                    lines.push(DiffLine {
                        kind: DiffKind::Context,
                        old_no: Some(i as u32 + 1),
                        new_no: Some(j as u32 + 1),
                        text: old[i].to_string(),
                    });
                }
                Op::Remove => {
                    old_count += 1;
                    removed += 1;
                    lines.push(DiffLine {
                        kind: DiffKind::Remove,
                        old_no: Some(i as u32 + 1),
                        new_no: None,
                        text: old[i].to_string(),
                    });
                }
                Op::Add => {
                    new_count += 1;
                    added += 1;
                    lines.push(DiffLine {
                        kind: DiffKind::Add,
                        old_no: None,
                        new_no: Some(j as u32 + 1),
                        text: new[j].to_string(),
                    });
                }
            }
        }
        // Start lines: the first old/new line in the hunk, or the line before when empty.
        let (_, first_i, first_j) = ops[start];
        let old_start = if old_count == 0 {
            first_i as u32
        } else {
            first_i as u32 + 1
        };
        let new_start = if new_count == 0 {
            first_j as u32
        } else {
            first_j as u32 + 1
        };
        hunks.push(Hunk {
            header: format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@"),
            lines,
        });
    }
    FileChange {
        path: path.to_string(),
        added,
        removed,
        hunks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_numbers_and_counts_are_consistent() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n";
        let new = "a\nb\nX\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\n";
        let d = unified_diff("f.txt", old, new);
        assert_eq!((d.added, d.removed), (2, 1));
        assert_eq!(d.hunks.len(), 2);
        assert_eq!(d.hunks[0].header, "@@ -1,6 +1,6 @@");
        assert_eq!(d.hunks[1].header, "@@ -10,3 +10,4 @@");
        for h in &d.hunks {
            let old_lines = h.lines.iter().filter(|l| l.kind != DiffKind::Add).count();
            assert!(h.header.contains(&format!(",{old_lines} ")));
        }
        let first_add = d.hunks[0]
            .lines
            .iter()
            .find(|l| l.kind == DiffKind::Add)
            .unwrap();
        assert_eq!(first_add.new_no, Some(3));
    }

    #[test]
    fn new_file_has_zero_old_range() {
        let d = unified_diff("new.rs", "", "one\ntwo\n");
        assert_eq!(d.hunks[0].header, "@@ -0,0 +1,2 @@");
        assert_eq!((d.added, d.removed), (2, 0));
    }
}
