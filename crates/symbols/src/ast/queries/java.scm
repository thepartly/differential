; Definitions: a type at file scope, and a method a class body declares. Java
; nests a top-level type directly under `program`, so that anchor does the work
; TypeScript's `export` gate does — a type declared inside another type is
; reached through its outer name and falls to the file-local rule below.
(program (class_declaration name: (identifier) @def))
(program (interface_declaration name: (identifier) @def))
(program (enum_declaration name: (identifier) @def))
(program (record_declaration name: (identifier) @def))
(program (annotation_type_declaration name: (identifier) @def))

; A method declared in a CLASS body is reached by that name from other files —
; the inherent-`impl` argument of ADR 0030, and the same rule Go and Kotlin
; already apply. A method in an `interface_body` is not one: every implementor
; declares the same name, which is the trait-method case ADR 0023 measured.
(class_body (method_declaration name: (identifier) @def))

; A call names its method in `name:` whether or not it has a receiver, so one
; pattern covers `plainCall()` and `w.methodCall()` both.
(method_invocation name: (identifier) @call)

; Named without being called: `Widget::render` hands a method to a `Runnable`,
; and `Config.TIMEOUT` reads a constant by path. Java spells a qualified name
; and a field read the same way, so this captures field reads too — the noise
; Go's `@ref` already carries, absorbed by the single-definer rule.
(method_reference (identifier) @ref)
(field_access field: (identifier) @ref)

(type_identifier) @type

; File-local names. Everything the file-scope rule turned down is still a
; binding: a nested type, an interface's method, a field, a local, and every
; parameter position (ADR 0030). Without these the catch-all below took them as
; READS, so a line declaring a name pointed at whatever else spelled it alike.
(class_declaration name: (identifier) @local_def)
(interface_declaration name: (identifier) @local_def)
(enum_declaration name: (identifier) @local_def)
(record_declaration name: (identifier) @local_def)
(annotation_type_declaration name: (identifier) @local_def)
(method_declaration name: (identifier) @local_def)
(variable_declarator name: (identifier) @local_def)
(formal_parameter name: (identifier) @local_def)
(catch_formal_parameter name: (identifier) @local_def)
(enhanced_for_statement name: (identifier) @local_def)
(inferred_parameters (identifier) @local_def)
(resource name: (identifier) @local_def)
(enum_constant name: (identifier) @local_def)
(type_parameter (type_identifier) @local_def)

(identifier) @local_ref
