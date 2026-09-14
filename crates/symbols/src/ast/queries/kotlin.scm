; Kotlin's `call_expression` has NO `function:` field and its
; `navigation_expression` names none of its children, which is why the generic
; field rule found no calls at all here. A query can still see the shape.
(class_declaration name: (identifier) @def)
(object_declaration name: (identifier) @def)
(source_file (function_declaration name: (identifier) @def))
(source_file (property_declaration (variable_declaration (identifier) @def)))

; A member function is declared in one class body and reached by that name from
; other files (ADR 0030).
(class_body (function_declaration name: (identifier) @def))

(call_expression (identifier) @call)
(call_expression (navigation_expression (identifier) @call .))

; Named without being called: `register(Routes::widgets)` and the like.
(navigation_expression (identifier) @ref .)

; A type position is `user_type`, whose child is the bare name.
(user_type (identifier) @type)

; File-local names: any property declaration, including the ones inside a
; function or a class body that the file-scope rule above drops (ADR 0030).
(variable_declaration (identifier) @local_def)

; A binding is a declaration wherever it is written. `variable_declaration`
; above already covers `for`, a lambda's parameters, a `when` subject and
; destructuring; these three are what it does not reach. The spec already calls
; every one of them file-local (spec/ordering.md), and without them the
; catch-all below took them as READS.
(parameter (identifier) @local_def)
(class_parameter (identifier) @local_def)
(catch_block (identifier) @local_def)

(identifier) @local_ref
