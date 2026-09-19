//! Tree-sitter readers, and what they share.
//!
//! Three readers sit on top of this module and none knows about the others.
//! [`tuned::AstSymbols`] has a hand-written query per language.
//! [`sfc::SfcSymbols`] runs the TypeScript query over a mask of a single-file
//! component's `<script>` blocks (ADR 0035). [`generic::AstTier2Symbols`] has
//! no query at all, and works from field names instead. They claim disjoint
//! sets of languages, so they never rank against each other.
//!
//! What they share is here: parsing, line numbering, [`run_query`] — the
//! meaning of every capture name, which the two readers that have a query both
//! read from — and [`is_prose`], the rule for deciding that a token is comment
//! or string rather than code.
//!
//! [`is_prose`] is the rule stated once, climbing from a token to the root. It
//! is only affordable per token when there are few of them, and since the
//! queries began capturing every identifier (ADR 0030) there are not — so both
//! readers carry the same two flags DOWN a cursor walk instead
//! ([`prose_tokens`] here, `generic::walk`'s own stack there), and `is_prose`
//! survives as the definition and as the tuned reader's fallback for a capture
//! that is not a token. The reason is measured: see
//! `deep_nesting_costs_neither_stack_nor_quadratic_time`.

pub mod generic;
pub mod sfc;
pub mod tuned;

use std::collections::HashSet;
use std::ops::Range;

use differential_engine::artefact::symbols::{FileSymbols, Symbol};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

/// Parse `content`, or `None` if the grammar cannot be installed.
///
/// A tree with errors in it is still returned. Tree-sitter recovers, and a
/// half-parsed file yields the symbols it did understand — which beats falling
/// through to a reader that understands nothing.
fn parse(language: &tree_sitter::Language, content: &[u8]) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;
    parser.parse(content, None)
}

/// How many lines `content` has, for sizing a `FileSymbols`.
fn line_count(content: &[u8]) -> usize {
    content.iter().filter(|&&b| b == b'\n').count() + 1
}

/// Is this token prose rather than code?
///
/// **Comments and strings are 44.5% of reference tokens** on a measured range,
/// and dropping them needs no query: every grammar gives comments and strings
/// their own node types, and names them with those words.
///
/// Strings need the one exception. `"${resolve(id)}"` holds a real call, so a
/// token is only prose if it reaches a string WITHOUT passing through an
/// interpolation on the way up.
fn is_prose(node: Node) -> bool {
    let mut current = node;
    let mut through_interpolation = false;
    while let Some(parent) = current.parent() {
        let kind = parent.kind();
        if kind.contains("comment") {
            return true;
        }
        if kind.contains("interpolation") || kind.contains("substitution") {
            through_interpolation = true;
        }
        if kind.contains("string") && !through_interpolation {
            return true;
        }
        current = parent;
    }
    false
}

/// The byte range of every prose TOKEN in the tree, in one linear pass.
///
/// [`is_prose`] answers the same question by climbing from a token to the root,
/// which costs depth per token. That was fine when a query handed back a
/// handful of captures. It stopped being fine when the queries gained
/// `(identifier) @local_ref` (ADR 0030) and started capturing every token in
/// the file: per-token × per-ancestor is the quadratic shape
/// `deep_nesting_costs_neither_stack_nor_quadratic_time` was written to catch,
/// and a minified bundle is where it shows.
///
/// Tokens only, because every capture in every query is one. A capture that
/// somehow is not stays correct — the caller falls back to [`is_prose`] for it.
fn prose_tokens(tree: &Tree) -> HashSet<Range<usize>> {
    /// What a node inherits from the level above it — the same two flags
    /// [`is_prose`] computes by climbing, carried down instead.
    #[derive(Clone, Copy)]
    struct Above {
        in_comment: bool,
        /// Inside a string, and no interpolation since: `"${resolve(id)}"`
        /// holds a real call, so an interpolation clears this.
        in_string: bool,
    }

    let mut out = HashSet::new();
    let mut cursor = tree.walk();
    let mut stack: Vec<Above> = vec![Above {
        in_comment: false,
        in_string: false,
    }];

    loop {
        let node = cursor.node();
        let kind = node.kind();
        let above = *stack.last().expect("the root entry is never popped");

        if node.child_count() == 0 {
            if above.in_comment || above.in_string {
                out.insert(node.byte_range());
            }
        } else if cursor.goto_first_child() {
            let interpolates = kind.contains("interpolation") || kind.contains("substitution");
            stack.push(Above {
                in_comment: above.in_comment || kind.contains("comment"),
                in_string: !interpolates && (above.in_string || kind.contains("string")),
            });
            continue;
        }
        // A sibling shares our depth, so it inherits the same entry; only
        // climbing out of a level pops one.
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return out;
            }
            stack.pop();
        }
    }
}

/// The line a node starts on, counting from 1.
fn line_of(node: Node) -> usize {
    node.start_position().row + 1
}

/// The token's byte range within its own line.
///
/// Tree-sitter's `Point::column` is already a BYTE offset into the row, so this
/// is the raw-line range `Site` asks for with no line table to build. A token
/// spanning rows — no query here captures one — reports its first row's tail
/// rather than a range that runs past the end of a line.
fn columns_of(node: Node) -> (u32, u32) {
    let start = node.start_position();
    let end = node.end_position();
    let from = start.column as u32;
    let to = if end.row == start.row {
        end.column as u32
    } else {
        from + (node.end_byte() - node.start_byte()) as u32
    };
    (from, to)
}

/// The last line of the declaration this NAME belongs to, counting from 1.
///
/// Every `@def` pattern in every tuned query captures the name identifier, and
/// its parent is the declaration that name introduces — `(function_item name:
/// (identifier) @def)`, `(variable_declarator name: (identifier) @def)`. So the
/// parent's end row is the body's end.
///
/// Where that is wrong it is wrong in the safe direction: a parent narrower
/// than the reader expected stops SHORT, showing fewer lines. It can never run
/// past the declaration into the next one, because a parent contains its child.
fn extent_of(node: Node) -> u32 {
    node.parent()
        .map_or(line_of(node), |p| p.end_position().row + 1) as u32
}

/// A node's text, or `None` if it is not UTF-8.
fn text_of<'a>(node: Node, content: &'a [u8]) -> Option<&'a [u8]> {
    content.get(node.byte_range())
}

/// One query capture, with everything the decision below needs.
///
/// A struct rather than the tuple this was: six fields travelling together is
/// what `clippy::type_complexity` exists to catch, and naming them is how the
/// veto below stays readable.
struct Capture<'a> {
    name: &'a str,
    /// Zero-based index into the parallel `FileSymbols` vectors.
    line: usize,
    /// Byte range in the FILE — the identity key the definition veto compares.
    range: Range<usize>,
    text: Vec<u8>,
    /// Byte range within the token's own line.
    columns: (u32, u32),
    through: u32,
}

/// Run one compiled query over `content` and decide what each capture means.
///
/// Shared by the two readers that HAVE a query — the tuned table, and the
/// single-file-component reader, which hands this the same TypeScript query
/// over a masked buffer (`sfc`). Nothing here knows which called it: a query
/// and a namespace are the whole input.
pub(super) fn run_query(
    language: &Language,
    query: &Query,
    content: &[u8],
    namespace: Vec<u8>,
) -> Option<FileSymbols> {
    let tree = parse(language, content)?;
    let lines = line_count(content);
    let mut out = FileSymbols {
        namespace,
        defines: vec![Vec::new(); lines],
        references: vec![Vec::new(); lines],
    };

    // Collect first, decide after. A definition site is also a type
    // mention — `struct Widget` matches both `@def` and `@type` — and query
    // matches arrive in no particular order, so the veto needs every
    // capture in hand.
    let prose = prose_tokens(&tree);
    let names = query.capture_names();
    let mut captured: Vec<Capture<'_>> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), content);
    while let Some(m) = matches.next() {
        for capture in m.captures() {
            let node = capture.node;
            // A token answers from the set; anything else — no query here
            // captures one — pays the ancestor walk.
            let prosaic = if node.child_count() == 0 {
                prose.contains(&node.byte_range())
            } else {
                is_prose(node)
            };
            if prosaic {
                continue;
            }
            let (Some(text), Some(line)) = (text_of(node, content), line_of(node).checked_sub(1))
            else {
                continue;
            };
            if line >= lines {
                continue;
            }
            let (from, to) = columns_of(node);
            let name = names[capture.index as usize];
            // **Only a definition pays for its extent.** `Node::parent`
            // walks DOWN from the root, so calling it once per capture is
            // an ancestor walk per token — the quadratic shape
            // `the_tuned_reader_survives_deep_nesting_too` exists to catch,
            // and it caught this: 20k levels went from seconds to 175.
            //
            // Since ADR 0030 there is one capture per token, and all but a
            // handful are `@local_ref`. A reference has no body to show, so
            // it has no reason to ask.
            let declares = name == "def" || name == "local_def";
            captured.push(Capture {
                name,
                line,
                range: node.byte_range(),
                text: text.to_vec(),
                columns: (from, to),
                through: if declares { extent_of(node) } else { 0 },
            });
        }
    }

    let ranges = |wanted: &str| -> HashSet<Range<usize>> {
        captured
            .iter()
            .filter(|c| c.name == wanted)
            .map(|c| c.range.clone())
            .collect()
    };
    let defined = ranges("def");
    let locally_defined = ranges("local_def");

    for c in captured {
        let Capture {
            name,
            line,
            range,
            text,
            columns: (from, to),
            through,
        } = c;
        // Definitions win: a class must never appear to consume the thing
        // it introduces. A file-scope definition also wins over the
        // file-local capture of the same token, which is how
        // `(variable_declarator …) @local_def` and its `(program …) @def`
        // sibling both stay in the query without fighting.
        let is_a_definition = defined.contains(&range) || locally_defined.contains(&range);
        // A definition carries its body's last line; a reference has no
        // body of its own, so it carries only its columns.
        match name {
            "def" => out.defines[line].push(Symbol::global(text).at(from, to).through(through)),
            "local_def" if !defined.contains(&range) => {
                out.defines[line].push(Symbol::local(text).at(from, to).through(through));
            }
            // Three spellings of one thing: the file consumes a name
            // that came from somewhere else. `@ref` is the one that is
            // neither a call nor a type — a function handed to a router
            // rather than invoked, an enum variant, a constant by path.
            "call" | "type" | "ref" if !is_a_definition => {
                out.references[line].push(Symbol::global(text).at(from, to));
            }
            // A call may be resolving a file-local binding or a global
            // one, and nothing here can say which. Both are recorded; the
            // graph keeps whichever finds a definer.
            "local_ref" if !is_a_definition => {
                out.references[line].push(Symbol::local(text).at(from, to));
            }
            _ => {}
        }
    }
    Some(out)
}
