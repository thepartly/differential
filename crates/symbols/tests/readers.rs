//! What each reader actually extracts, per language.
//!
//! Every snippet carries the same six things, so the table below reads the same
//! way for all of them:
//!
//! - a plain call and a method call — both wanted,
//! - a type used in a signature — wanted,
//! - a mention inside a comment and one inside a string — both noise,
//! - a declaration that names no usable symbol, and a method — neither is a
//!   definition, and counting them is what made 64% of one corpus range's
//!   dependency edges false.

use differential_engine::artefact::symbols::{FileSymbols, Scope, Symbol, SymbolSource};
use differential_symbols::{AstSymbols, AstTier2Symbols, NaiveSymbols, SfcSymbols};
use sha1::{Digest, Sha1};

/// One reader's answer, split by how far each name reaches.
///
/// The two `local_` sets are the ones a name may only be compared against
/// inside its own file (ADR 0030). Keeping them apart here is the point: a
/// test that means "this is a name other files can use" must not pass because
/// the identifier turned up in the file-local set.
struct Read {
    defines: Vec<String>,
    references: Vec<String>,
    local_defines: Vec<String>,
    local_references: Vec<String>,
}

fn flatten(rows: &[Vec<Symbol>], want: Scope) -> Vec<String> {
    let mut out: Vec<String> = rows
        .iter()
        .flatten()
        .filter(|s| s.scope == want)
        .map(|s| String::from_utf8_lossy(&s.name).into_owned())
        .collect();
    out.sort();
    out.dedup();
    out
}

fn read(reader: &dyn SymbolSource, path: &[u8], src: &str) -> Read {
    assert!(
        reader.priority(path).is_some(),
        "{} does not claim {}",
        reader.fingerprint(),
        String::from_utf8_lossy(path)
    );
    let s: FileSymbols = reader
        .file_symbols(path, src.as_bytes())
        .expect("the reader claimed this file");
    Read {
        defines: flatten(&s.defines, Scope::Global),
        references: flatten(&s.references, Scope::Global),
        local_defines: flatten(&s.defines, Scope::File),
        local_references: flatten(&s.references, Scope::File),
    }
}

/// One line's answer, both ways, for a test about an OCCURRENCE.
///
/// The sets above are over the whole file, so they cannot say "this token here
/// is a declaration and not a read" — a name declared on one line and read on
/// the next is honestly in both. A binding site is exactly that question, so it
/// is asked a line at a time.
struct Line {
    defines: Vec<String>,
    references: Vec<String>,
}

fn read_line(reader: &dyn SymbolSource, path: &[u8], src: &str, holding: &str) -> Line {
    let s: FileSymbols = reader
        .file_symbols(path, src.as_bytes())
        .expect("the reader claimed this file");
    let at = src
        .lines()
        .position(|l| l.contains(holding))
        .unwrap_or_else(|| panic!("no line holds {holding:?}"));
    let names = |rows: &[Vec<Symbol>]| -> Vec<String> {
        rows.get(at)
            .map(|row| {
                row.iter()
                    .map(|s| String::from_utf8_lossy(&s.name).into_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    Line {
        defines: names(&s.defines),
        references: names(&s.references),
    }
}

fn has(set: &[String], want: &[&str]) {
    for w in want {
        assert!(set.contains(&w.to_string()), "missing {w:?} in {set:?}");
    }
}

fn lacks(set: &[String], unwanted: &[&str]) {
    for u in unwanted {
        assert!(!set.contains(&u.to_string()), "unwanted {u:?} in {set:?}");
    }
}

// ------------------------------------------------------------ tuned readers

#[test]
fn every_tuned_query_compiles_against_its_pinned_grammar() {
    // A query and its grammar are both ours and both pinned, so a mismatch is a
    // bug to fix rather than a state to ship. This is the loud failure that a
    // hand-written tree walk could not give us.
    let failures = AstSymbols::new();
    assert!(
        failures.failures().is_empty(),
        "queries that would not compile: {:?}",
        failures.failures()
    );
    // The SFC reader compiles the TypeScript query against the same pinned
    // grammar, and answers nothing at all if it cannot.
    assert!(
        SfcSymbols::new().ready(),
        "the SFC reader's query would not compile against its grammar"
    );
}

/// Rust: modules and TRAIT methods are not definitions, inherent ones are.
///
/// `fn from` in `impl From<u8> for Widget` shares its name with every other
/// conversion in the tree — the ambiguity ADR 0023 measured. `fn serve` in
/// `impl Widget` shares its name with nothing, and callers in other files
/// reach it by that name (ADR 0030). `!trait` is the whole of the difference.
#[test]
fn rust_takes_inherent_methods_and_still_refuses_modules_and_trait_methods() {
    let r = read(
        &AstSymbols::new(),
        b"src/lib.rs",
        r#"
mod template;
pub struct Widget;
impl From<u8> for Widget { fn from(v: u8) -> Self { Widget } }
impl Widget { pub fn serve(&self) -> u8 { 0 } }
// mentions NoiseA
pub fn render(w: Widget) -> u8 { plain_call(); w.method_call(); let s = "NoiseB"; 0 }
pub fn wire() { route(other::module::handler); }
"#,
    );
    has(&r.defines, &["Widget", "render", "serve"]);
    lacks(
        &r.defines,
        &["template", "from", "NoiseA", "NoiseB", "method_call"],
    );
    has(&r.references, &["plain_call", "method_call", "Widget"]);
    // Handed to a router rather than invoked. Only the callee position was
    // captured before, so a registration table drew no edge at all.
    has(&r.references, &["handler"]);
    lacks(&r.references, &["NoiseA", "NoiseB", "template"]);
}

#[test]
fn python_reads_annotations_as_types() {
    let r = read(
        &AstSymbols::new(),
        b"app.py",
        r#"
class Widget:
    def method_on_class(self): pass
# mentions NoiseA
def render(w: Widget) -> Widget:
    plain_call()
    w.method_call()
    s = "NoiseB"
    return w
"#,
    );
    has(&r.defines, &["Widget", "render", "method_on_class"]);
    has(&r.references, &["plain_call", "method_call", "Widget"]);
    lacks(&r.references, &["NoiseA", "NoiseB"]);
}

/// Go: a method's receiver names exactly one type, so the method is a
/// definition (ADR 0030) — the analogue of Rust's inherent `impl`, and Go has
/// no trait-impl case to separate out.
#[test]
fn go_reads_selectors_and_takes_methods() {
    let r = read(
        &AstSymbols::new(),
        b"main.go",
        r#"
package p
// mentions NoiseA
type Widget struct{}
func (w Widget) MethodOnType() {}
func Render(w Widget) string { plainCall(); w.MethodCall(); return "NoiseB" }
func Wire() { mux.Handle("/x", handlers.Listing) }
"#,
    );
    has(&r.defines, &["Widget", "Render", "MethodOnType"]);
    lacks(&r.defines, &["NoiseA", "NoiseB"]);
    has(&r.references, &["plainCall", "MethodCall", "Widget"]);
    // Handed over rather than called. Go spells this the same way as a struct
    // field read, so the capture takes both — ADR 0030 states the cost.
    has(&r.references, &["Listing"]);
    lacks(&r.references, &["NoiseA", "NoiseB"]);
}

#[test]
fn typescript_reads_members_and_exports() {
    let r = read(
        &AstSymbols::new(),
        b"app.ts",
        r#"
// mentions NoiseA
export interface Widget { n: number }
export function render(w: Widget): string { plainCall(); w.methodCall(); return "NoiseB" }
"#,
    );
    has(&r.defines, &["Widget", "render"]);
    has(&r.references, &["plainCall", "methodCall", "Widget"]);
    lacks(&r.references, &["NoiseA", "NoiseB"]);
}

#[test]
fn kotlin_needed_a_query_and_now_reads_both_call_shapes() {
    // The generic field rule found NO calls here: Kotlin's `call_expression`
    // has no `function:` field, and `navigation_expression` names none of its
    // children. That is why Kotlin earned a query.
    let r = read(
        &AstSymbols::new(),
        b"Main.kt",
        r#"
// mentions NoiseA
class Widget { fun serve(): Int { return 0 } }
fun render(w: Widget): Int { plainCall(); w.methodCall(); val s = "NoiseB"; return 0 }
"#,
    );
    has(&r.defines, &["Widget", "render", "serve"]);
    has(&r.references, &["plainCall", "methodCall", "Widget"]);
    lacks(&r.references, &["NoiseA", "NoiseB"]);
}

/// Java: a class body's method is a definition, an interface's is not.
///
/// The field rules could not tell those apart — they saw one
/// `method_declaration` under one `declaration`-ish parent and called every
/// method file-local, so Java drew no cross-file method edge at all. The query
/// makes the distinction ADR 0030 draws: `render` is declared in one class and
/// callers elsewhere name it exactly, while `paint` is declared again by every
/// implementor of `Renderer`.
#[test]
fn java_takes_a_class_method_and_still_refuses_an_interface_one() {
    let r = read(
        &AstSymbols::new(),
        b"Main.java",
        r#"
// mentions NoiseA
interface Renderer { String paint(); }
class Widget implements Renderer {
  private String label;
  void methodOnType() {}
  public String paint() { return label; }
  String render(Widget w) { plainCall(); w.methodCall(); return "NoiseB"; }
}
"#,
    );
    has(
        &r.defines,
        &["Widget", "Renderer", "render", "methodOnType"],
    );
    // `paint` is declared twice here and by every other implementor too. The
    // interface's declaration is file-local; the class's is not, which is the
    // same answer Go and Kotlin give a method.
    has(&r.local_defines, &["paint", "label"]);
    lacks(&r.defines, &["label"]);
    has(&r.references, &["plainCall", "methodCall", "Widget"]);
    lacks(&r.references, &["NoiseA", "NoiseB"]);
}

/// C#: every type is a definition wherever it is declared, and a method is one
/// only when a class, struct or record declares it.
///
/// C# has no `type_identifier` node and no file-scope anchor: a nested type is
/// reached as `Outer.Inner` and a namespaced one through its `using`, so the
/// query gates on the DECLARING node rather than on where it sits. Both bodies
/// are spelled `declaration_list`, which is why the owner is named.
#[test]
fn csharp_names_its_types_by_their_declaration_and_not_by_a_type_node() {
    let r = read(
        &AstSymbols::new(),
        b"Widget.cs",
        r#"
// mentions NoiseA
namespace Acme;
interface IRenderer { string Paint(); }
public record Point(int X, int Y);
public class Widget : IRenderer, IBox<Point>, System.IDisposable, MyNs.IRepo<Point>
{
    private string label;
    public string Paint() { return label; }
    public string Render(Widget other, List<string> rows)
    {
        PlainCall();
        other.MethodCall();
        return "NoiseB";
    }
    System.Collections.Generic.Queue<Point> Batch(MyNs.Plain p, MyNs.Repository<int> repo) { return null; }
    System.String Describe() { return label; }
    Marker Clone() { return null; }
    Stack<Point> Many() { return null; }
}
"#,
    );
    has(
        &r.defines,
        &["Widget", "IRenderer", "Point", "Render", "Paint"],
    );
    // A type reaches the graph through the field it sits in, not through a
    // node kind — C# has no `type_identifier`. Three positions need patterns:
    // the `type:` wildcard, `returns:` (its own field, which the wildcard never
    // reaches) and `base_list` (no field at all). Each is spelled four ways —
    // bare, generic, qualified by namespace, and qualified AND generic, the
    // last needing the tail unwrapped twice. Twelve patterns, twelve cases.
    //
    // Every name here appears exactly once in the snippet. That second
    // condition is the one that bites: `List` was the first choice for the
    // `returns:` qualified-generic case, and `List<string>` is already a
    // parameter type, so the assertion was satisfied by the `type:` wildcard
    // and deleting the `returns:` pattern still passed. A test that cannot
    // fail looks exactly like a test that passes.
    has(
        &r.references,
        &[
            "Widget",      // type: bare
            "List",        // type: generic
            "Plain",       // type: qualified
            "Repository",  // type: qualified + generic
            "Marker",      // returns: bare
            "Stack",       // returns: generic
            "String",      // returns: qualified
            "Queue",       // returns: qualified + generic
            "IRenderer",   // base_list: bare
            "IBox",        // base_list: generic
            "IDisposable", // base_list: qualified
            "IRepo",       // base_list: qualified + generic
        ],
    );
    // The namespace parts of a qualified name are not types, and stay noise.
    lacks(&r.references, &["System", "Collections", "Generic", "MyNs"]);
    lacks(&r.references, &["NoiseA", "NoiseB"]);
}

/// Swift: a type's method is a definition, a protocol's requirement is not.
///
/// Swift spells `class`, `struct` and `enum` all as `class_declaration`, so one
/// pattern takes all three — and its `call_expression` names none of its
/// children, which is Kotlin's shape and the reason a query is the only way to
/// read either language.
#[test]
fn swift_takes_a_type_method_and_reads_a_call_with_no_named_callee() {
    let r = read(
        &AstSymbols::new(),
        b"Widget.swift",
        r#"
// mentions NoiseA
protocol Renderer { func paint() -> String }
struct Point { let x: Int }
class Widget: Renderer {
    let label: String
    func paint() -> String { return label }
    func render(other: Widget) -> String { plainCall(); other.methodCall(); return "NoiseB" }
}
"#,
    );
    has(
        &r.defines,
        &["Widget", "Renderer", "Point", "render", "paint"],
    );
    has(
        &r.references,
        &["plainCall", "methodCall", "Widget", "String"],
    );
    lacks(&r.references, &["NoiseA", "NoiseB"]);
}

/// PHP: a class's method is a definition; an interface's and a trait's are not.
///
/// All three bodies are spelled `declaration_list`, so the owner has to be
/// named. A file-scope `const` is a name others can use and a class constant is
/// not, which is the one place PHP needs the file-scope anchor at all.
#[test]
fn php_separates_a_class_method_from_an_interface_and_a_trait_one() {
    let r = read(
        &AstSymbols::new(),
        b"Widget.php",
        r#"<?php
// mentions NoiseA
interface Renderer { public function paint(): string; }
trait Loggable { public function log(): void {} }
const LIMIT = 3;
class Widget implements Renderer {
    private string $label;
    const CAP = 1;
    public function paint(): string { return $this->label; }
    public function render(Widget $other): string {
        plainCall();
        $other->methodCall();
        Helper::staticCall();
        return Mode::Fast . "NoiseB";
    }
}
"#,
    );
    has(
        &r.defines,
        &["Widget", "Renderer", "Loggable", "LIMIT", "render", "paint"],
    );
    // A trait's method and a class constant are both reached through the thing
    // that owns them (ADR 0030).
    has(&r.local_defines, &["log", "CAP", "label"]);
    lacks(&r.defines, &["log", "CAP"]);
    has(
        &r.references,
        &["plainCall", "methodCall", "staticCall", "Widget", "Helper"],
    );
    // A constant read by path gives up both halves: the class is a type, the
    // case is a name consumed without being called.
    has(&r.references, &["Mode", "Fast"]);
    lacks(&r.references, &["NoiseA", "NoiseB"]);
}

/// Zig: `pub` is the whole predicate, exactly as `export` is in TypeScript.
///
/// A type is a `const` bound to a container body and a value is a `const` bound
/// to anything else, so the SHAPE of a declaration says nothing about who can
/// reach it — which is why a private `@import` binding, shaped exactly like a
/// type, must not be taken. Doing so would make every importing file the
/// definer of the name it imported: the `mod template;` false definition
/// ADR 0030 measured. A `pub` re-export is a different statement, and is taken.
#[test]
fn zig_gates_on_pub_and_so_refuses_a_private_import_binding() {
    let r = read(
        &AstSymbols::new(),
        b"widget.zig",
        r#"
// mentions NoiseA
const Helper = @import("helper.zig");
pub const ReExport = @import("other.zig");
pub const MAX = 100;
const private_thing = 7;
pub const Alias = Elsewhere;

pub const Widget = struct {
    label: []const u8,

    pub fn render(self: Widget, other: Widget) void {
        plainCall();
        other.methodCall();
        Helper.staticCall();
        const s = "NoiseB";
        drop(s);
    }

    fn privateHelper() void {}
};

pub fn freeFunction(a: i32) i32 { return a; }
fn privateFunction() void {}
"#,
    );
    // A `pub` name is reachable from another file whatever it is bound to — a
    // container body, a plain value, another name, or a re-exported import.
    has(
        &r.defines,
        &[
            "Widget",
            "render",
            "freeFunction",
            "MAX",
            "Alias",
            "ReExport",
        ],
    );
    // The anchor is what stops `pub const Alias = Elsewhere;` defining the name
    // on its right-hand side as well.
    lacks(&r.defines, &["Elsewhere"]);
    // A private import binding is file-local, so `Helper.staticCall()` still
    // draws its edge — inside this file, where it is honestly known.
    lacks(&r.defines, &["Helper", "private_thing", "privateHelper"]);
    lacks(&r.defines, &["privateFunction"]);
    has(&r.local_defines, &["Helper", "label", "self", "other", "s"]);
    has(&r.local_references, &["Helper"]);
    has(
        &r.references,
        &["plainCall", "methodCall", "staticCall", "Widget"],
    );
    lacks(&r.references, &["NoiseA", "NoiseB"]);
}

/// A Vue SFC is read as the script it is.
///
/// No tree-sitter Vue grammar parses a `<script>` body — it is one `raw_text`
/// node, resolved by an injection the Rust query engine does not do. So the
/// reader masks the file down to its script blocks and runs the TypeScript
/// query over the result. Line numbers and columns are the FILE's, which is the
/// whole reason for masking rather than parsing a substring and adding offsets.
#[test]
fn a_vue_component_is_read_through_its_script_and_not_its_template() {
    let src = r#"<template>
  <ChildWidget :label="title" />
</template>

<script setup lang="ts">
// mentions NoiseA
import { computed } from 'vue'
const count = ref(0)
const formatted = computed(() => formatLabel(count.value))
const caption = `a ${resolve(count)} b`
function bump(step: number) { count.value += step; return "NoiseB" }
</script>

<style scoped>
.panel { color: NoiseC; }
</style>
"#;
    let r = read(&SfcSymbols::new(), b"Panel.vue", src);
    has(&r.references, &["computed", "ref", "formatLabel"]);
    has(
        &r.local_defines,
        &["count", "formatted", "bump", "step", "caption", "computed"],
    );
    // Comments and strings inside the script are dropped by the reader, the
    // same way they are for every other language — the mask is not what does
    // that. An interpolation still holds real code, so `${resolve(count)}`
    // keeps its call.
    lacks(&r.references, &["NoiseA", "NoiseB"]);
    has(&r.references, &["resolve"]);
    // The template and the style are masked out, so nothing in either can
    // become a symbol — `ChildWidget` included.
    lacks(&r.references, &["ChildWidget", "NoiseC", "panel", "title"]);
    lacks(&r.local_defines, &["ChildWidget", "panel"]);

    // The mask is what keeps a line number honest: `bump` is on the file's
    // ninth line, not the script's fifth.
    let line = read_line(&SfcSymbols::new(), b"Panel.vue", src, "function bump");
    assert!(
        line.defines.iter().any(|n| n == "bump"),
        "bump lands on its own line: {line:?}",
        line = line.defines
    );
}

/// A component with no script block is DECLINED, not answered empty.
///
/// Answering with an empty `FileSymbols` would claim the file and state that it
/// has no symbols. Declining says something different and true — this reader
/// cannot read it — and hands it to the floor, exactly as a failed parse does.
#[test]
fn a_template_only_component_falls_to_the_floor() {
    let sfc = SfcSymbols::new();
    let path = b"Static.vue".as_slice();
    let src = b"<template>\n  <p>hello</p>\n</template>\n";
    assert!(sfc.priority(path).is_some(), "it claims every .vue");
    assert!(
        sfc.file_symbols(path, src).is_none(),
        "and declines the ones it cannot read"
    );
    assert!(NaiveSymbols.priority(path).is_some(), "the floor takes it");
}

/// A classic `<script>` block is an ES module, and its named exports are taken.
///
/// Nothing special makes this work: masking puts the block at the top level of
/// a program, and the TypeScript query's `export` gate does the rest. It is
/// the half of the story the component's own missing name tends to hide — a
/// `.vue` file can and does define names other files import.
#[test]
fn a_classic_script_blocks_named_exports_are_definitions_like_any_module_s() {
    let r = read(
        &SfcSymbols::new(),
        b"Panel.vue",
        r#"<template>
  <div>{{ label }}</div>
</template>

<script lang="ts">
export const PANEL_LIMIT = 10
export interface PanelProps { title: string }
export function formatTitle(t: string) { return t.trim() }
export default { inheritAttrs: false }
</script>

<script setup lang="ts">
const label = formatTitle('x')
</script>
"#,
    );
    has(&r.defines, &["PANEL_LIMIT", "PanelProps", "formatTitle"]);
    // `export default { … }` is an anonymous object literal, so it names
    // nothing — and a `<script setup>` binding is not an export at all.
    has(&r.local_defines, &["label", "t"]);
    lacks(&r.defines, &["label", "inheritAttrs", "Panel"]);
}

/// The component's OWN name is never exported, and nothing global would read it
/// if it were.
///
/// `<script setup>` has the compiler generate the default export and a classic
/// block writes `export default { … }`, an anonymous object literal — so
/// neither gives the component an identifier. The name importers use is the
/// FILE's, and deriving it from the path was considered and dropped: an
/// importer records `import Child from './Child.vue'` as a FILE-LOCAL binding
/// (ADR 0030), and unlike a named export there is no later USE to supply the
/// global reference, because the only place a component is used is a
/// `<template>` and that is masked out. A global definition with no global
/// consumer is the false-definition shape ADR 0023 measured, so this pins the
/// absence rather than leaving it to be re-litigated. ADR 0035.
#[test]
fn a_component_draws_no_incoming_edge_because_an_import_is_file_local() {
    let component = read(
        &SfcSymbols::new(),
        b"ChildWidget.vue",
        "<script setup lang=\"ts\">\nconst n = 1\n</script>\n",
    );
    assert!(
        component.defines.is_empty(),
        "a component names nothing globally: {:?}",
        component.defines
    );
    let importer = read(
        &AstSymbols::new(),
        b"page.ts",
        "import ChildWidget from './ChildWidget.vue'\nexport const wrap = () => ChildWidget\n",
    );
    has(&importer.local_defines, &["ChildWidget"]);
    lacks(&importer.references, &["ChildWidget"]);
}

// ------------------------------------------------------ the field-rule reader

/// The field rules still take JavaScript, C and C++, and this is what they buy
/// there: symbols from the tree, with every value conservatively file-local.
#[test]
fn javascript_reads_through_field_names_with_no_query() {
    let r = read(
        &AstTier2Symbols::new(),
        b"widget.js",
        r#"
// mentions NoiseA
class Widget {
  methodOnType() {}
  render(w) { plainCall(); w.methodCall(); return "NoiseB"; }
}
"#,
    );
    has(&r.defines, &["Widget"]);
    lacks(&r.defines, &["render", "methodOnType"]);
    has(&r.references, &["plainCall", "methodCall"]);
    lacks(&r.references, &["NoiseA", "NoiseB"]);
}

#[test]
fn the_tuned_reader_outranks_the_field_reader_and_they_never_overlap() {
    let tuned = AstSymbols::new();
    let fields = AstTier2Symbols::new();
    // Disjoint by construction: a language has a query or it does not.
    for path in [
        b"src/lib.rs".as_slice(),
        b"Main.kt",
        b"app.ts",
        b"main.go",
        b"Main.java",
        b"a.cs",
        b"a.swift",
        b"a.php",
        b"a.zig",
    ] {
        assert!(tuned.priority(path).is_some());
        assert!(fields.priority(path).is_none(), "both claimed {path:?}");
    }
    for path in [b"a.c".as_slice(), b"a.cpp", b"a.js"] {
        assert!(tuned.priority(path).is_none(), "both claimed {path:?}");
        assert!(fields.priority(path).is_some());
    }
    // And the SFC reader takes `.vue` alone. It ranks with the tuned reader
    // because it runs a query too, so a second claimant on one path would put
    // the two at 9 apiece and let registration order decide.
    let sfc = SfcSymbols::new();
    assert!(sfc.priority(b"Panel.vue").is_some());
    for path in [
        b"src/lib.rs".as_slice(),
        b"app.ts",
        b"a.js",
        b"Main.java",
        b"a.swift",
    ] {
        assert!(sfc.priority(path).is_none(), "the SFC reader took {path:?}");
    }
    assert!(tuned.priority(b"Panel.vue").is_none());
    assert!(fields.priority(b"Panel.vue").is_none());
    // The ranking, on the ONE path where it could ever be consulted. This used
    // to compare the two readers over two different files — a contest the
    // loops above prove can never happen, so it reduced to comparing two
    // unrelated constants. `SymbolReaders::of_file` only ranks readers that
    // both claim the SAME path, and today no path is claimed twice.
    let both: Vec<&[u8]> = [
        b"src/lib.rs".as_slice(),
        b"Main.kt",
        b"app.ts",
        b"main.go",
        b"Main.java",
        b"a.c",
        b"a.cpp",
        b"a.cs",
        b"a.js",
    ]
    .into_iter()
    .filter(|p| tuned.priority(p).is_some() && fields.priority(p).is_some())
    .collect();
    assert!(
        both.is_empty(),
        "these paths are claimed twice, so the ranking now decides them: {both:?}"
    );
    // And when a language does gain a query, the tuned reader must win it.
    assert!(
        AstSymbols::new().priority(b"x.kt") > AstTier2Symbols::new().priority(b"x.kt").or(Some(0)),
        "a tuned reader must outrank the field reader wherever both could claim"
    );
}

/// The floor is the AST readers' fallback when a parse fails, so it has to
/// claim everything they claim. It kept a list of its own and the list lagged:
/// `.pyi`, `.mts` and `.cts` were the tuned reader's and nobody else's, so a
/// failed parse there would have yielded no symbols rather than crude ones.
#[test]
fn the_floor_stands_under_every_file_the_ast_readers_claim() {
    let tuned = AstSymbols::new();
    let fields = AstTier2Symbols::new();
    let sfc = SfcSymbols::new();
    for path in [
        b"a.rs".as_slice(),
        b"a.pyi",
        b"a.mts",
        b"a.cts",
        b"a.tsx",
        b"a.kts",
        b"a.js",
        b"a.java",
        b"a.h",
        b"a.hh",
        b"a.cs",
        b"a.swift",
        b"a.php",
        b"a.zig",
        b"Panel.vue",
    ] {
        let name = String::from_utf8_lossy(path);
        let above = tuned
            .priority(path)
            .or(fields.priority(path))
            .or(sfc.priority(path));
        assert!(
            above.is_some(),
            "{name} is nobody's, so it proves nothing here"
        );
        assert!(
            NaiveSymbols.priority(path).is_some(),
            "an AST reader claims {name} and the floor does not"
        );
        assert!(
            NaiveSymbols.priority(path) < above,
            "the floor must rank below the reader it stands under on {name}"
        );
    }
}

/// Deep nesting must cost neither stack nor quadratic time.
///
/// This reader takes JavaScript, C and C++, where a minified bundle
/// or a generated literal makes AST depth track nesting. Two separate hazards
/// live there, and this one test catches both:
///
/// - **Stack.** A recursive walk takes a frame per node. 20,000 levels is far
///   past what a 2 MiB test thread survives, so a return to recursion aborts
///   rather than merely slowing down.
/// - **Time.** `Node::parent` walks down from the root every call, so asking
///   each node for its parent costs depth per node. Writing it that way made
///   this test exceed two minutes at this depth; carrying the parent's kind
///   down on the
///   walk's own stack makes it finish in milliseconds.
///
/// **An identifier at every level, not just the deepest one.** Nesting bare
/// brackets would leave one token at maximum depth, and a per-token ancestor
/// walk — the shape `is_prose` has — would still pass. `f(f(f(…)))` puts a
/// token in callee position at each level, so any rule that climbs from a token
/// to the root goes quadratic here too.
///
/// Tree-sitter parses this input in milliseconds, so anything slower is ours.
#[test]
fn deep_nesting_costs_neither_stack_nor_quadratic_time() {
    const DEPTH: usize = 20_000;
    let mut src = String::with_capacity(DEPTH * 6 + 32);
    src.push_str("const deep = ");
    for _ in 0..DEPTH {
        src.push_str("wrap(");
    }
    src.push_str("widgetMaker()");
    for _ in 0..DEPTH {
        src.push(')');
    }
    src.push_str(";\n");

    // The "nor quadratic time" half of the name, measured. Without a clock
    // this test only proved the stack held: a reintroduced per-token ancestor
    // walk took minutes and still passed green, because the Rust harness
    // imposes no timeout of its own.
    //
    // The budget is deliberately loose — thirty seconds on a machine that
    // does this in milliseconds. It is a trip-wire for a quadratic walk, not
    // a benchmark, so an unloaded laptop and a busy CI runner both clear it.
    let started = std::time::Instant::now();
    let symbols = AstTier2Symbols::new()
        .file_symbols(b"bundle.js", src.as_bytes())
        .expect("the reader claimed this file and must answer");
    let took = started.elapsed();
    assert!(
        took < std::time::Duration::from_secs(30),
        "reading {DEPTH} levels took {took:?}: something walks from a token to \
         the root, which is quadratic in the depth"
    );

    // The innermost call is still found, so the walk reached the bottom rather
    // than stopping part way.
    let references = flatten(&symbols.references, Scope::Global);
    has(&references, &["widgetMaker", "wrap"]);
}

// ------------------------------------------------------------ file-local names

/// A binding is a declaration in every position that introduces one.
///
/// `if let Some(first)` DECLARES `first`; it does not read it. While only
/// `let` and a parameter counted, the catch-all `(identifier) @local_ref` took
/// every other binding as a read — so the reviewer's line underlined the name
/// it was declaring and the float pointed at whatever else in the file spelled
/// it the same way.
#[test]
fn rust_reads_every_binding_position_as_a_declaration() {
    const SRC: &str = r#"
pub fn read_them(rows: Vec<u8>) -> u8 {
    if let Some(first) = head(&rows) {
        drop(first);
    }
    for each in &rows { drop(each); }
    let pair = (1u8, 2u8);
    let (left, right) = pair;
    let Widget { size } = make();
    let add = |lifted: u8| lifted + 1;
    match rows.len() { other => drop(other) }
    drop(size); drop(left); drop(right); add(0)
}
"#;
    let r = read(&AstSymbols::new(), b"src/bind.rs", SRC);
    has(
        &r.local_defines,
        &["first", "each", "left", "right", "size", "lifted"],
    );
    // A later line that genuinely READS one still says so.
    has(&r.local_references, &["size", "left", "right"]);
    // A match arm's child is a variant path far more often than a binding, so
    // no arm captures one: `other` stays a read.
    lacks(&r.local_defines, &["other"]);

    // The reported line, asked as an OCCURRENCE: `first` is declared here and
    // is not read here. Before this, the catch-all made it a read, so the row
    // underlined the name it was declaring.
    let l = read_line(
        &AstSymbols::new(),
        b"src/bind.rs",
        SRC,
        "if let Some(first)",
    );
    has(&l.defines, &["first"]);
    lacks(&l.references, &["first"]);
    // The line's genuine read is untouched: `rows` is being consumed.
    has(&l.references, &["rows"]);
}

/// The case convention is what separates a binding from a unit variant.
///
/// Rust writes both as a bare `identifier` inside a pattern, and no grammar can
/// tell them apart without resolving names. Capturing `None` as a declaration
/// would point a later use of it at a match arm.
#[test]
fn a_rust_pattern_declares_a_binding_and_never_a_variant() {
    let r = read(
        &AstSymbols::new(),
        b"src/variant.rs",
        r#"
pub fn pick(got: Result<Option<u8>, u8>) -> u8 {
    if let Ok(None) = got { return LIMIT; }
    if let Ok(Some(taken)) = got { return taken; }
    0
}
"#,
    );
    has(&r.local_defines, &["taken"]);
    lacks(&r.local_defines, &["None", "Ok", "Some", "LIMIT"]);
}

/// Python: a loop target, an `as` alias, a walrus and an unpacking all declare.
#[test]
fn python_reads_every_binding_position_as_a_declaration() {
    let r = read(
        &AstSymbols::new(),
        b"bind.py",
        r#"
def read_them(rows, limit=10):
    for each in rows:
        drop(each)
    with open_it() as handle:
        drop(handle)
    try:
        drop(limit)
    except ValueError as err:
        drop(err)
    left, right = rows
    if (found := lookUpName()):
        drop(found)
    drop(left)
    drop(right)
"#,
    );
    has(
        &r.local_defines,
        &["each", "handle", "err", "left", "right", "found", "limit"],
    );
    has(&r.local_references, &["each", "left", "right"]);
}

/// Go: a range clause, a type switch's alias and every parameter declare.
#[test]
fn go_reads_every_binding_position_as_a_declaration() {
    let r = read(
        &AstSymbols::new(),
        b"bind.go",
        r#"
package p
func ReadThem(rows []string, rest ...string) {
	for index, each := range rows {
		drop(index)
		drop(each)
	}
	switch taken := any(rows).(type) {
	default:
		drop(taken)
	}
	drop(rest)
}
"#,
    );
    has(
        &r.local_defines,
        &["rows", "rest", "index", "each", "taken"],
    );
    has(&r.local_references, &["rows", "each", "taken"]);
}

/// TypeScript: destructuring, a catch parameter and a bare arrow parameter
/// all declare.
#[test]
fn typescript_reads_every_binding_position_as_a_declaration() {
    let r = read(
        &AstSymbols::new(),
        b"bind.ts",
        r#"
export function readThem(rows: string[]) {
  const [first, second] = rows;
  const { size: measured, ...rest } = shapeOf(rows);
  const add = lifted => lifted + 1;
  try {
    drop(first);
  } catch (err) {
    drop(err);
  }
  for (const key in rows) { drop(key); }
  return add(second) + measured + rest;
}
"#,
    );
    has(
        &r.local_defines,
        &[
            "first", "second", "measured", "rest", "lifted", "err", "key",
        ],
    );
    has(&r.local_references, &["first", "second", "measured"]);
}

/// Kotlin: a function parameter and a catch parameter declare.
///
/// `variable_declaration` already reached `for`, a lambda's parameters, a
/// `when` subject and destructuring, so these two are the whole gap.
#[test]
fn kotlin_reads_every_binding_position_as_a_declaration() {
    let r = read(
        &AstSymbols::new(),
        b"Bind.kt",
        r#"
class Holder(val size: Int)
fun readThem(rows: List<String>) {
    for (each in rows) { drop(each) }
    try { drop(rows) } catch (err: Exception) { drop(err) }
}
"#,
    );
    has(&r.local_defines, &["rows", "each", "err", "size"]);
    has(&r.local_references, &["rows", "each"]);
}

/// Java: a `for`-each name, a catch parameter, a try-with-resources name, an
/// inferred lambda parameter and a varargs declarator all declare.
///
/// `variable_declarator` and `formal_parameter` reach the ordinary cases and
/// the varargs one; these five are the positions that sit outside both, and
/// without them the catch-all took a declaration for a READ.
#[test]
fn java_reads_every_binding_position_as_a_declaration() {
    let r = read(
        &AstSymbols::new(),
        b"Bind.java",
        r#"
class Bind<T> {
  void readThem(String... rest) throws Exception {
    for (String row : rest) { drop(row); }
    try (var res = open()) { drop(res); }
    catchIt((a, b) -> drop(a));
    catchIt(bare -> drop(bare));
  }
  void catchIt(Object f) {
    try { drop(f); } catch (RuntimeException err) { drop(err); }
  }
}
"#,
    );
    has(
        &r.local_defines,
        &["rest", "row", "res", "a", "b", "bare", "err", "f", "T"],
    );
    has(
        &r.local_references,
        &["rest", "row", "res", "a", "bare", "err"],
    );
}

/// C#: `foreach`, a catch declaration, an `is` pattern, a tuple deconstruction
/// and a bare lambda parameter all declare.
///
/// The bare one is why `implicit_parameter` is captured as a whole node rather
/// than through a field: in `x => x + 1` the grammar gives the parameter no
/// child to name.
#[test]
fn csharp_reads_every_binding_position_as_a_declaration() {
    let r = read(
        &AstSymbols::new(),
        b"Bind.cs",
        r#"
class Bind
{
    void ReadThem<T>(List<string> rows, T item)
    {
        foreach (var row in rows) { Drop(row); }
        try { Drop(item); } catch (Exception err) { Drop(err); }
        if (rows is List<string> cast) { Drop(cast); }
        using (var res = Open()) { Drop(res); }
        (int a, int b) = Pair();
        Func<int, int> f = x => x + 1;
        Drop(a); Drop(b); Drop(f);
    }
}
"#,
    );
    has(
        &r.local_defines,
        &[
            "rows", "item", "row", "err", "cast", "res", "a", "b", "f", "x", "T",
        ],
    );
    has(
        &r.local_references,
        &["rows", "row", "err", "cast", "res", "a", "x"],
    );
}

/// Swift: `bound_identifier:` is the one field every binding position shares,
/// and two of them hang it off the statement rather than off a pattern.
///
/// A property, a `for` item and a `catch` error put it under a `(pattern)`;
/// `if let` and `guard let` put it directly under the statement. The wildcard
/// is what reaches both shapes, and without it the catch-all took every one of
/// them for a READ.
#[test]
fn swift_reads_every_binding_position_as_a_declaration() {
    let r = read(
        &AstSymbols::new(),
        b"Bind.swift",
        r#"
func readThem(rows: [String]) {
    var total = 0
    for row in rows { drop(row) }
    do { try open() } catch let err { drop(err) }
    if let cast = rows.first { drop(cast) }
    guard let kept = rows.last else { return }
    rows.forEach { item in drop(item) }
    drop(total)
    drop(kept)
}
"#,
    );
    has(
        &r.local_defines,
        &["rows", "total", "row", "err", "cast", "kept", "item"],
    );
    has(
        &r.local_references,
        &["rows", "row", "err", "cast", "kept", "item"],
    );
}

/// PHP declares a variable by assigning to it, so the assignment's left side IS
/// the binding position — there is no `let` to key on.
///
/// `foreach` is the awkward one: the grammar gives neither the collection nor
/// the binding a field, so the two-child form is reached by anchoring past the
/// collection and the `as $k => $v` form through its `pair`.
#[test]
fn php_reads_every_binding_position_as_a_declaration() {
    let r = read(
        &AstSymbols::new(),
        b"bind.php",
        r#"<?php
function readThem(array $rows, int ...$rest) {
    $total = 0;
    foreach ($rows as $row) { drop($row); }
    foreach ($rows as $key => $val) { drop($key); drop($val); }
    try { open(); } catch (RuntimeException $err) { drop($err); }
    $fn = fn($x) => $x;
    $cb = function ($y) use ($total) { return $y; };
    [$a, $b] = pair();
    drop($rest); drop($a); drop($b);
}
"#,
    );
    has(
        &r.local_defines,
        &[
            "rows", "rest", "total", "row", "key", "val", "err", "fn", "cb", "x", "y", "a", "b",
        ],
    );
    has(
        &r.local_references,
        &["rows", "row", "key", "val", "err", "x", "a"],
    );
}

/// Zig: a `|payload|` is the binding `if`, `while`, `for` and `catch` all
/// share, and a declaration's name is its first child with no field to key on.
#[test]
fn zig_reads_every_binding_position_as_a_declaration() {
    let r = read(
        &AstSymbols::new(),
        b"bind.zig",
        r#"
pub fn readThem(rows: [][]const u8) void {
    var total: usize = 0;
    for (rows) |row| { drop(row); }
    while (next()) |item| { drop(item); }
    if (maybe()) |cast| { drop(cast); }
    drop(total);
}
"#,
    );
    has(&r.local_defines, &["rows", "total", "row", "item", "cast"]);
    has(
        &r.local_references,
        &["rows", "row", "item", "cast", "total"],
    );
}

/// The change that made this whole distinction necessary, in miniature.
///
/// A React component is `export const Panel = …`, its dependencies are
/// `<Child/>` and `{label}`, and none of those three shapes drew an edge before
/// (ADR 0030). The label is declared INSIDE the component, so it is file-local:
/// the reviewer still needs to read the declaration before the three lines that
/// render it, and nothing else in the change can say so.
#[test]
fn tsx_reads_components_jsx_and_the_locals_a_render_consumes() {
    let r = read(
        &AstSymbols::new(),
        b"panel.tsx",
        r#"
// mentions NoiseA
import { Child } from './child';
export interface PanelProps { fallback: string }
type Local = { n: number };
export const Panel = ({ fallback }: PanelProps) => {
  const label = lookUpName() ?? fallback;
  const unused = "NoiseB";
  return <Child title={label}>{label}</Child>;
};
"#,
    );
    // An exported file-scope `const` is a definition, exactly as an exported
    // `function Panel()` is.
    has(&r.defines, &["Panel", "PanelProps"]);
    // An UNEXPORTED type is not, however file-scope it looks. `type FormData =
    // …` in a test file was once the only "definition" of that name in a
    // change, and every file mentioning it linked to the test.
    lacks(&r.defines, &["Local"]);
    has(&r.local_defines, &["Local"]);
    // The component it renders is consumed, and the props type is used.
    has(&r.references, &["Child", "PanelProps", "lookUpName"]);
    // The binding inside the body reaches only this file — and the JSX that
    // reads it says so.
    has(&r.local_defines, &["label", "fallback", "unused"]);
    has(&r.local_references, &["label"]);
    // Neither set takes anything from a comment or a string.
    lacks(&r.defines, &["NoiseA", "NoiseB", "label"]);
    lacks(&r.references, &["NoiseA", "NoiseB"]);
    lacks(&r.local_defines, &["NoiseA", "NoiseB"]);
    lacks(&r.local_references, &["NoiseA", "NoiseB"]);
}

/// A `.ts` file is the tuned reader's, and it now captures every identifier.
///
/// The sibling test below covers the field-rule reader. This one exists because
/// `(identifier) @local_ref` turned a handful of captures per file into one per
/// token, and `is_prose` answers each of them by climbing to the root — which
/// is a per-token ancestor walk, the exact shape that went quadratic once
/// before. A minified `.ts` bundle is where it would show.
#[test]
fn the_tuned_reader_survives_deep_nesting_too() {
    const DEPTH: usize = 20_000;
    let mut src = String::with_capacity(DEPTH * 6 + 32);
    src.push_str("const deep = ");
    for _ in 0..DEPTH {
        src.push_str("wrap(");
    }
    src.push_str("widgetMaker()");
    for _ in 0..DEPTH {
        src.push(')');
    }
    src.push_str(";\n");

    let started = std::time::Instant::now();
    let symbols = AstSymbols::new()
        .file_symbols(b"bundle.ts", src.as_bytes())
        .expect("the reader claimed this file and must answer");
    let took = started.elapsed();
    assert!(
        took < std::time::Duration::from_secs(30),
        "reading {DEPTH} levels took {took:?}: something walks from a token to \
         the root once per capture, which is quadratic in the depth"
    );
    has(
        &flatten(&symbols.references, Scope::Global),
        &["widgetMaker", "wrap"],
    );
}

/// Every definition names its own columns, and covers its own body (ADR 0032).
///
/// Both halves matter to a reader who wants to SEE the dependency rather than
/// be told it exists: the columns are what a highlight lands on, and `through`
/// is how much of the declaration there is to show.
#[test]
fn a_definition_carries_its_columns_and_its_body() {
    struct Case {
        path: &'static [u8],
        src: &'static str,
        name: &'static str,
        /// Byte columns of the name, then the declaration's last line.
        want: (u32, u32, u32),
    }

    let tuned = AstSymbols::new();
    // Four languages, four declaration shapes, one rule: the captured name's
    // parent is the declaration, so its end row is the body's end.
    let cases = [
        Case {
            path: b"a.rs",
            src: "pub fn serve(x: u8) -> u8 {\n    x + 1\n}\n",
            name: "serve",
            want: (7, 12, 3),
        },
        Case {
            path: b"a.py",
            src: "def handle(e):\n    log(e)\n    return 1\n",
            name: "handle",
            want: (4, 10, 3),
        },
        Case {
            path: b"a.go",
            src: "package p\nfunc Serve(x int) int {\n\treturn x\n}\n",
            name: "Serve",
            want: (5, 10, 4),
        },
        Case {
            path: b"a.ts",
            src: "export function lookUpName(): string {\n  return '';\n}\n",
            name: "lookUpName",
            want: (16, 26, 3),
        },
    ];
    for c in cases {
        let where_ = String::from_utf8_lossy(c.path).into_owned();
        let s = tuned
            .file_symbols(c.path, c.src.as_bytes())
            .expect("the tuned reader claims these");
        let found = s
            .defines
            .iter()
            .flatten()
            .find(|y| y.name == c.name.as_bytes())
            .unwrap_or_else(|| panic!("{} was not defined in {where_}", c.name));
        assert_eq!(
            (found.site.start, found.site.end, found.site.through),
            c.want,
            "{} in {where_}",
            c.name
        );
    }
}

/// A column is a RAW byte offset, not a display column.
///
/// The TUI expands tabs before drawing, so a reader that reported expanded
/// columns would mis-highlight every tab-indented file — and the failure is
/// silent, because the highlight still lands on SOMETHING. Go is the case that
/// matters: gofmt indents with tabs.
#[test]
fn a_column_counts_raw_bytes_and_never_expands_a_tab() {
    let tuned = AstSymbols::new();
    let s = tuned
        .file_symbols(b"a.go", b"package p\nfunc f() {\n\tplain()\n}\n")
        .expect("the tuned reader claims .go");
    let call = s
        .references
        .iter()
        .flatten()
        .find(|y| y.name == b"plain")
        .expect("the call is a reference");
    // One tab, then the name. Raw: column 1. Expanded at any tab width it
    // would be 4 or 8, so this fails loudly if the offsets ever change base.
    assert_eq!(
        (call.site.start, call.site.end),
        (1, 6),
        "a tab is one byte, whatever it draws as"
    );
}

/// The crude reader reports columns and refuses to invent an extent.
///
/// A regex has no tree to ask how far a declaration runs. Zero says so; a guess
/// at the next blank line would be a snippet that is confidently wrong.
#[test]
fn the_crude_reader_places_a_name_but_claims_no_body() {
    let s = NaiveSymbols
        .file_symbols(b"a.rb", b"class Widget\n  def serve\n  end\nend\n")
        .expect("the crude reader claims .rb");
    let def = s
        .defines
        .iter()
        .flatten()
        .find(|y| y.name == b"Widget")
        .expect("`class Widget` defines Widget");
    assert_eq!((def.site.start, def.site.end), (6, 12));
    assert_eq!(def.site.through, 0, "a regex cannot see an extent");
}

/// A query's version reaches the grouping cache key, so editing a pattern
/// without bumping the version serves a stale grouping for a graph that moved.
///
/// **Patterns, not prose.** The hash is taken over the `.scm` with its comment
/// and blank lines removed, because those cannot change an answer and a bump
/// costs every cached grouping in every checkout. That is also why the version
/// is a hand-written string rather than the hash itself: hashing the file into
/// the cache key would cold the cache to fix a typo in a comment.
///
/// Update both together: change a pattern, bump the `-vN` in `tuned.rs`, then
/// paste the hash this test prints.
///
/// (A `;` inside a query string literal would be mistaken for a comment here.
/// No query uses one; a predicate like `(#eq? @x ";")` would need this to
/// strip comments with a real tokeniser instead.)
#[test]
fn every_query_version_pins_its_patterns() {
    const PINNED: &[(&str, &str)] = &[
        ("rust-v4", "96ca20c967959d58aba41990b2f7ec03a00703d2"),
        ("python-v4", "d0dcef048323407d43643a013e70ed1437bdd39f"),
        ("go-v4", "05c59b455dc516f49383cfe7d3577ee54f254ffe"),
        ("typescript-v4", "a959775f12a0d9f24e79423d12a92fe11aa46bdd"),
        ("tsx-v4", "7d991606e98d9ae2aee744df90d114c9448aab3d"),
        ("kotlin-v4", "64b5b5aa082f00fc71ee8e5500577d532a30f0cf"),
        ("java-v1", "659f2843fd4028823b099498ab401528d2033455"),
        ("csharp-v1", "e51a1379f9f464306274ee71a155e7bbc8c10082"),
        ("swift-v1", "24295c3bae3c4bef7e7da00f93261f9fd78b0c83"),
        ("php-v1", "ca86f321d951314dc8c67fbfb39b7a8e8e33a456"),
        ("zig-v1", "1bdf88b0506ceee988bb85711f78da95654cd2b4"),
    ];
    let actual: Vec<(String, String)> = AstSymbols::queries()
        .into_iter()
        .map(|(version, text)| {
            let patterns: Vec<&str> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with(';'))
                .collect();
            let mut hasher = Sha1::new();
            hasher.update(patterns.join("\n").as_bytes());
            (version.to_string(), hex::encode(hasher.finalize()))
        })
        .collect();
    let pinned: Vec<(String, String)> = PINNED
        .iter()
        .map(|(v, h)| (v.to_string(), h.to_string()))
        .collect();
    assert_eq!(
        actual, pinned,
        "a query's patterns moved without its version: bump the `-vN` in \
         tuned.rs and paste the hashes above"
    );
}

/// Every reader's fingerprint, pinned to what it actually ANSWERS.
///
/// `every_query_version_pins_its_patterns` covers a `.scm` edit. Nothing
/// covered a change to the readers' Rust, and the gap was not theoretical: the
/// crude reader began answering with a namespace (ADR 0031) — changing which
/// cross-file symbols match, for the several languages it is the only reader
/// for — and kept `naive-v1`. A reviewer caught it, which is not a mechanism.
///
/// The fingerprint reaches the grouping cache key, so a reader that answers
/// differently and keeps its version serves a grouping built from a graph that
/// has since moved. This makes that impossible: hash each reader's answer over
/// fixed samples, and pin it beside the version.
///
/// **A tree-sitter grammar upgrade fails this too, and should.** A new grammar
/// can change extraction, which changes the graph, which must cold the cache —
/// and nothing else in the tree would have noticed.
///
/// **Adding a SAMPLE moves a hash without any reader having changed**, and that
/// move must NOT bump a version: no cached grouping is stale, and bumping would
/// cold every one of them for nothing. Three `.swift`, `.php` and `.zig`
/// samples moved the floor's hash exactly this way — the floor answers those
/// files as it always did, it is simply now asked. The version and the hash
/// mean different things, which is why both are pinned.
///
/// To update: change the reader, bump its version, paste the hashes below.
#[test]
fn every_reader_fingerprint_pins_its_answers() {
    // One sample per reader tier, kept deliberately small — this pins CHANGE,
    // not coverage. What each reader extracts is tested above, by name.
    const SAMPLES: &[(&str, &str)] = &[
        (
            "a.rs",
            "pub struct W;\nimpl W { pub fn serve(&self) -> u8 { call(); 0 } }\n",
        ),
        (
            "a.py",
            "class W:\n    def m(self): pass\ndef r(w: W): plain(); w.meth()\n",
        ),
        (
            "a.go",
            "package p\ntype W struct{}\nfunc (w W) M() { plain(); w.Meth() }\n",
        ),
        (
            "a.ts",
            "export interface P { n: number }\nexport const f = (p: P) => call(p);\n",
        ),
        ("a.tsx", "export const C = () => <Child n={1} />;\n"),
        (
            "A.kt",
            "class W { fun serve(): Int { plain(); return 0 } }\n",
        ),
        ("a.js", "export const f = (p) => call(p);\n"),
        (
            "A.java",
            "class W { String r(W w) { plain(); return w.meth(); } }\n",
        ),
        ("a.c", "int r(struct W *w) { return plain(w); }\n"),
        (
            "a.swift",
            "class W { func serve() -> Int { plain(); return 0 } }\n",
        ),
        (
            "a.php",
            "<?php\nclass W { function r(W $w) { plain(); return $w->meth(); } }\n",
        ),
        (
            "a.zig",
            "pub const W = struct {\n    pub fn serve(self: W) void { plain(); }\n};\n",
        ),
        (
            "P.vue",
            "<template><p/></template>\n<script setup lang=\"ts\">\nconst n = call();\n</script>\n",
        ),
        ("a.rb", "class W\n  def serve\n    plain_call\n  end\nend\n"),
        ("q.sql", "select id from widgets where owner_id = $1\n"),
    ];

    /// One reader's answer for one sample, as bytes that change iff it does.
    fn answer(reader: &dyn SymbolSource, path: &str, src: &str) -> String {
        let Some(s) = reader.file_symbols(path.as_bytes(), src.as_bytes()) else {
            return "declined\n".to_string();
        };
        let mut out = format!("ns={}\n", String::from_utf8_lossy(&s.namespace));
        for (line, (defines, references)) in s.defines.iter().zip(&s.references).enumerate() {
            // The SITE is hashed too. It is not part of any query, and the
            // graph never reads it — so a change that moved only a column or
            // an extent would otherwise pass this test while serving a stale
            // grouping for a reader that answers differently (ADR 0032).
            let show = |kind: &str, syms: &[Symbol]| -> String {
                syms.iter()
                    .map(|y| {
                        let scope = if y.scope == Scope::Global { "g" } else { "f" };
                        format!(
                            "{kind}{scope}:{}@{}-{}+{}",
                            String::from_utf8_lossy(&y.name),
                            y.site.start,
                            y.site.end,
                            y.site.through
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            };
            if !defines.is_empty() || !references.is_empty() {
                out.push_str(&format!(
                    "{line} {} {}\n",
                    show("d", defines),
                    show("r", references)
                ));
            }
        }
        out
    }

    let readers: Vec<Box<dyn SymbolSource>> = vec![
        Box::new(AstSymbols::new()),
        Box::new(SfcSymbols::new()),
        Box::new(AstTier2Symbols::new()),
        Box::new(NaiveSymbols),
    ];
    let actual: Vec<(String, String)> = readers
        .iter()
        .map(|reader| {
            let mut hasher = Sha1::new();
            for (path, src) in SAMPLES {
                // Only what this reader claims: a reader must not be pinned to
                // another's files, or every bump would cascade.
                if reader.priority(path.as_bytes()).is_some() {
                    hasher.update(path.as_bytes());
                    hasher.update(answer(reader.as_ref(), path, src).as_bytes());
                }
            }
            (reader.fingerprint(), hex::encode(hasher.finalize()))
        })
        .collect();

    const PINNED: &[(&str, &str)] = &[
        (
            "ast-tuned-v2[csharp-v1,go-v4,java-v1,kotlin-v4,php-v1,python-v4,rust-v4,swift-v1,tsx-v4,typescript-v4,zig-v1]",
            "04f3b5060d27eae51f040e9d3695d235da67f696",
        ),
        (
            "sfc-v1[typescript-v4]",
            "2b7681f9556c805df338fc214fea83b36db03667",
        ),
        ("ast-fields-v4", "ecbe41bf43ddfcef4c6883e3c26b12bd2dbe4416"),
        ("naive-v4", "b360aa01832f2e39cc50179dedc7fabc6e8c6c19"),
    ];
    let pinned: Vec<(String, String)> = PINNED
        .iter()
        .map(|(v, h)| (v.to_string(), h.to_string()))
        .collect();
    assert_eq!(
        actual, pinned,
        "a reader answers differently: bump its version and paste the hashes above"
    );
}
