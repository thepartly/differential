; Definitions. A method's receiver names exactly one type, so the method name is
; declared in one place and callers in other files reach it by that name — the
; same argument as an inherent `impl` in Rust (ADR 0030).
(type_declaration (type_spec name: (type_identifier) @def))
(source_file (function_declaration name: (identifier) @def))
(source_file (const_declaration (const_spec name: (identifier) @def)))
(method_declaration name: (field_identifier) @def)

(call_expression function: (identifier) @call)
(call_expression function: (selector_expression field: (field_identifier) @call))

; Named without being called: `mux.Handle("/x", handlers.Listing)` hands a
; function over. Go spells a qualified name and a struct field read the same
; way, so this also captures field reads — noise the single-definer rule has to
; absorb, and the reason this one is called out in ADR 0030 as unmeasured.
(selector_expression field: (field_identifier) @ref)

(type_identifier) @type

; File-local names: short declarations, vars, and methods reached through
; their receiver (ADR 0030).
(short_var_declaration left: (expression_list (identifier) @local_def))
(var_spec name: (identifier) @local_def)
(const_spec name: (identifier) @local_def)

; A binding is a declaration wherever it is written, not only after `:=`. A
; range clause, a type switch's alias, a receive and every parameter introduce a
; name, and the spec already calls them file-local (spec/ordering.md). Without
; these the catch-all below took them as READS, so a line declaring a name
; pointed at whatever else in the file spelled it the same way.
;
; Go needs no convention rule here as Rust does: none of these positions can
; hold anything but a new name.
(range_clause left: (expression_list (identifier) @local_def))
(type_switch_statement alias: (expression_list (identifier) @local_def))
(receive_statement left: (expression_list (identifier) @local_def))
(parameter_declaration name: (identifier) @local_def)
(variadic_parameter_declaration name: (identifier) @local_def)

(identifier) @local_ref
(field_identifier) @local_ref
