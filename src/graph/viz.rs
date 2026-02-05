use std::collections::HashMap;

use crate::core::repo::RepoId;

pub fn render_tree(
    roots: &[RepoId],
    edges: &HashMap<RepoId, Vec<RepoId>>,
    labels: &HashMap<RepoId, String>,
) -> String {
    let mut out = String::new();
    for (idx, root) in roots.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(
            labels
                .get(root)
                .map(String::as_str)
                .unwrap_or_else(|| root.as_str()),
        );
        out.push('\n');
        let mut path = Vec::new();
        render_tree_children(root, edges, labels, "", &mut path, &mut out);
    }
    out
}

pub fn render_flat(
    roots: &[RepoId],
    edges: &HashMap<RepoId, Vec<RepoId>>,
    labels: &HashMap<RepoId, String>,
) -> String {
    let mut out = String::new();
    for (idx, root) in roots.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(
            labels
                .get(root)
                .map(String::as_str)
                .unwrap_or_else(|| root.as_str()),
        );
        out.push('\n');
        let mut path = Vec::new();
        render_flat_children(root, edges, labels, 1, &mut path, &mut out);
    }
    out
}

pub fn render_dot(
    edges: &HashMap<RepoId, Vec<RepoId>>,
    labels: &HashMap<RepoId, String>,
) -> String {
    let mut out = String::from("digraph harmonia {\n");
    for (node, label) in labels {
        let escaped = escape_dot_label(label);
        out.push_str(&format!(
            "  \"{}\" [label=\"{}\"];\n",
            node.as_str(),
            escaped
        ));
    }
    for (from, deps) in edges {
        for dep in deps {
            out.push_str(&format!(
                "  \"{}\" -> \"{}\";\n",
                from.as_str(),
                dep.as_str()
            ));
        }
    }
    out.push_str("}\n");
    out
}

fn render_tree_children(
    node: &RepoId,
    edges: &HashMap<RepoId, Vec<RepoId>>,
    labels: &HashMap<RepoId, String>,
    prefix: &str,
    path: &mut Vec<RepoId>,
    out: &mut String,
) {
    let mut children = edges.get(node).cloned().unwrap_or_else(Vec::new);
    children.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    for (idx, child) in children.iter().enumerate() {
        let is_last = idx + 1 == children.len();
        out.push_str(prefix);
        out.push_str(if is_last { "`-- " } else { "|-- " });
        let label = labels
            .get(child)
            .map(String::as_str)
            .unwrap_or_else(|| child.as_str());
        out.push_str(label);
        if path.iter().any(|id| id == child) {
            out.push_str(" (cycle)");
            out.push('\n');
            continue;
        }
        out.push('\n');
        path.push(child.clone());
        let mut next_prefix = prefix.to_string();
        if is_last {
            next_prefix.push_str("    ");
        } else {
            next_prefix.push_str("|   ");
        }
        render_tree_children(child, edges, labels, &next_prefix, path, out);
        path.pop();
    }
}

fn render_flat_children(
    node: &RepoId,
    edges: &HashMap<RepoId, Vec<RepoId>>,
    labels: &HashMap<RepoId, String>,
    depth: usize,
    path: &mut Vec<RepoId>,
    out: &mut String,
) {
    let mut children = edges.get(node).cloned().unwrap_or_else(Vec::new);
    children.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    for child in children {
        for _ in 0..depth {
            out.push_str("  ");
        }
        let label = labels
            .get(&child)
            .map(String::as_str)
            .unwrap_or_else(|| child.as_str());
        out.push_str(label);
        if path.iter().any(|id| id == &child) {
            out.push_str(" (cycle)");
            out.push('\n');
            continue;
        }
        out.push('\n');
        path.push(child.clone());
        render_flat_children(&child, edges, labels, depth + 1, path, out);
        path.pop();
    }
}

fn escape_dot_label(label: &str) -> String {
    label.replace('"', "\\\"")
}
