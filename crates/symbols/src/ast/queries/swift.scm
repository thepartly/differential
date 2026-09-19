; Swift's grammar spells `class`, `struct` and `enum` all as
; `class_declaration`, distinguished only by the body node, so one pattern takes
; all three. A type declared inside another is reached through its outer name,
; which is why the file-scope anchor is here exactly as it is in Java.
(source_file (class_declaration name: (type_identifier) @def))
(source_file (protocol_declaration name: (type_identifier) @def))
(source_file (typealias_declaration name: (type_identifier) @def))
(source_file (function_declaration name: (simple_identifier) @def))

; A method a type's body declares is reached by that name from other files
; (ADR 0030). A `protocol_function_declaration` is not: every conformer declares
; it, which is the trait-method case.
(class_body (function_declaration name: (simple_identifier) @def))
(enum_class_body (function_declaration name: (simple_identifier) @def))

; Swift's `call_expression` names none of its children — the callee is simply
; the first child, and a method call reaches its name through a
; `navigation_suffix`. That is Kotlin's shape, and the same reason a query is
; the only way to read either language.
(call_expression (simple_identifier) @call)
(call_expression (navigation_expression suffix: (navigation_suffix suffix: (simple_identifier) @call)))

; Named without being called: `let r = Widget.render` hands a method over, and
; `Config.limit` reads a constant by path. Swift spells a member read the same
; way, so this carries those too — Go's known edge, in this language.
(navigation_expression suffix: (navigation_suffix suffix: (simple_identifier) @ref))

(user_type (type_identifier) @type)

; File-local names. `bound_identifier:` is the one field every binding position
; shares — a property, a `for` item, a `catch` error, an `if let` and a
; `guard let` all use it, and the last two hang it off the statement rather than
; off a pattern. The wildcard is what reaches both shapes without listing them.
(_ bound_identifier: (simple_identifier) @local_def)
(class_declaration name: (type_identifier) @local_def)
(protocol_declaration name: (type_identifier) @local_def)
(typealias_declaration name: (type_identifier) @local_def)
(function_declaration name: (simple_identifier) @local_def)
(protocol_function_declaration name: (simple_identifier) @local_def)
(parameter name: (simple_identifier) @local_def)
(lambda_parameter name: (simple_identifier) @local_def)
(enum_entry name: (simple_identifier) @local_def)

(simple_identifier) @local_ref
