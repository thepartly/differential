; Definitions. A method lives in one class body, so its name is declared in one
; place and callers reach it by that name (ADR 0030) — the same argument as an
; inherent `impl` in Rust, though Python is duck-typed and two classes may well
; answer to the same method name. The single-definer rule drops those.
(class_definition name: (identifier) @def)
(module (function_definition name: (identifier) @def))
(module (decorated_definition definition: (function_definition name: (identifier) @def)))
(class_definition body: (block (function_definition name: (identifier) @def)))
(class_definition body: (block (decorated_definition definition:
  (function_definition name: (identifier) @def))))

(call function: (identifier) @call)
(call function: (attribute attribute: (identifier) @call))

; Named without being called: `router.add(views.widgets)`. As in Go, an
; attribute read and a qualified name are one syntax here.
(attribute attribute: (identifier) @ref)

; Python has no type nodes, so the annotation's `type:` field is the signal.
(typed_parameter type: (type (identifier) @type))
(function_definition return_type: (type (identifier) @type))
(typed_parameter type: (type (subscript value: (identifier) @type)))

; File-local names: assignments, parameters and methods — reached through
; their class, or not at all outside this file (ADR 0030).
(assignment left: (identifier) @local_def)
(parameters (identifier) @local_def)
(typed_parameter (identifier) @local_def)

; A binding is a declaration wherever it is written, not only on the left of a
; plain `=`. A loop target, an `as` alias, a walrus, an unpacking and a
; parameter with a default all introduce a name, and the spec already calls
; every one of them file-local (spec/ordering.md). Without these the catch-all
; below took them as READS, so a line declaring a name pointed at whatever else
; in the file spelled it the same way.
(for_statement left: (identifier) @local_def)
(for_statement left: (pattern_list (identifier) @local_def))
(for_statement left: (tuple_pattern (identifier) @local_def))
(for_in_clause left: (identifier) @local_def)
(for_in_clause left: (pattern_list (identifier) @local_def))
(for_in_clause left: (tuple_pattern (identifier) @local_def))
; One node for `except E as err` and for `with open(p) as f`.
(as_pattern alias: (as_pattern_target (identifier) @local_def))
(named_expression name: (identifier) @local_def)
(assignment left: (pattern_list (identifier) @local_def))
(assignment left: (tuple_pattern (identifier) @local_def))
(assignment left: (list_pattern (identifier) @local_def))
(default_parameter name: (identifier) @local_def)
(typed_default_parameter name: (identifier) @local_def)
(lambda_parameters (identifier) @local_def)

(identifier) @local_ref
