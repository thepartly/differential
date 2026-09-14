; Definitions: names this file introduces that other files can use.
; NOT `mod x;` — that names a module, not a usable symbol. That was counted by
; the regex this replaces, and `mod template;` alone produced 21% of one range's
; edges.
;
; An INHERENT `impl` method is one, and a TRAIT `impl` method is not (ADR 0030).
; `fn from` in `impl From<X> for Y` is reached through the trait and shares its
; name with every other conversion in the tree — that is the ambiguity ADR 0023
; measured. `impl Service { pub async fn load_batch }` shares its name with
; nothing: it is the one place that name is declared, and callers in other files
; reach it by that name. `!trait` is the whole of the difference.
(struct_item name: (type_identifier) @def)
(enum_item name: (type_identifier) @def)
(union_item name: (type_identifier) @def)
(trait_item name: (type_identifier) @def)
(type_item name: (type_identifier) @def)
(macro_definition name: (identifier) @def)
(source_file (const_item name: (identifier) @def))
(source_file (static_item name: (identifier) @def))
(source_file (function_item name: (identifier) @def))
(impl_item !trait body: (declaration_list (function_item name: (identifier) @def)))
(source_file (mod_item body: (declaration_list (function_item name: (identifier) @def))))

; Calls, plain and through a receiver.
(call_expression function: (identifier) @call)
(call_expression function: (scoped_identifier name: (identifier) @call))
(call_expression function: (field_expression field: (field_identifier) @call))
(macro_invocation macro: (identifier) @call)

; Named by path without being called: `route(api::widgets::handler)`
; hands a function over rather than invoking it, and registration tables are
; built of exactly that. Capturing only the callee position missed every one.
(scoped_identifier name: (identifier) @ref)

; Types used, including through a path.
(type_identifier) @type

; File-local names: everything the file-scope rule above deliberately drops —
; a binding inside a function, a parameter, a method reached through its type.
; Scoped to this file, they can only order classes within it (ADR 0030).
(let_declaration pattern: (identifier) @local_def)
(parameter pattern: (identifier) @local_def)
(function_item name: (identifier) @local_def)
(const_item name: (identifier) @local_def)
(static_item name: (identifier) @local_def)

; A binding is a declaration wherever it is written, not only after `let`.
; `if let Some(x)`, a `for` pattern, a closure parameter and every destructuring
; shape introduce a name too, and the spec already calls every one of them
; file-local (spec/ordering.md). Without these the catch-all below took them as
; READS, so a line declaring a name underlined it and pointed the reader at
; whatever else in the file spelled it the same way.
;
; Rust writes a binding and a unit variant with the same node: the `None` in
; `Ok(None)` is a bare `identifier` exactly as the `a` in `Ok(a)` is. No
; grammar can separate those without resolving names, so the case convention
; does it — a binding is snake_case, a variant or a const is not. It is the one
; heuristic in this file, and it is why a bare `match_pattern` arm is absent:
; there the child is a variant path more often than a binding.
((tuple_struct_pattern type: (_) (identifier) @local_def)
 (#not-match? @local_def "^[A-Z]"))
((tuple_pattern (identifier) @local_def)
 (#not-match? @local_def "^[A-Z]"))
((slice_pattern (identifier) @local_def)
 (#not-match? @local_def "^[A-Z]"))
((ref_pattern (identifier) @local_def)
 (#not-match? @local_def "^[A-Z]"))
((mut_pattern (identifier) @local_def)
 (#not-match? @local_def "^[A-Z]"))
((captured_pattern (identifier) @local_def)
 (#not-match? @local_def "^[A-Z]"))
((field_pattern pattern: (identifier) @local_def)
 (#not-match? @local_def "^[A-Z]"))
; `Widget { name }` binds `name`, and the shorthand has a node of its own that
; neither catch-all below reaches.
(field_pattern (shorthand_field_identifier) @local_def)
(let_condition pattern: (identifier) @local_def)
(for_expression pattern: (identifier) @local_def)
(closure_parameters (identifier) @local_def)

(identifier) @local_ref
(field_identifier) @local_ref
