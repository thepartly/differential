; Zig has no type declaration: a type is a `const` bound to a `struct`, `enum`,
; `union` or `opaque` body, and the grammar gives the bound name no field — it
; is simply the declaration's first child, which is what the anchor pins.
;
; The bound BODY is what this gates on, and it has to. `const Helper =
; @import("helper.zig")` has the same shape as a type, and taking it would make
; every importing file the definer of the name it imported — the `mod template;`
; false definition ADR 0030 measured at 64% of one range's edges. An import
; binding falls to the file-local rule below, where it draws every edge it
; honestly can inside its own file.
(source_file (variable_declaration . (identifier) @def
  [(struct_declaration) (enum_declaration) (union_declaration) (opaque_declaration)]))
(source_file (function_declaration name: (identifier) @def))

; A function a container's body declares is reached through that container's
; name from other files (ADR 0030). Zig has no interface to exempt: a method is
; a plain function inside a struct.
(struct_declaration (function_declaration name: (identifier) @def))
(union_declaration (function_declaration name: (identifier) @def))
(enum_declaration (function_declaration name: (identifier) @def))
(opaque_declaration (function_declaration name: (identifier) @def))

(call_expression function: (identifier) @call)
(call_expression function: (field_expression member: (identifier) @call))

; Named without being called: `Helper.staticCall` handed over, or a constant
; read by path. Zig spells a struct-field read the same way, so this carries
; those too — Go's known edge, in this language.
(field_expression member: (identifier) @ref)

; Zig has no type node either: a type is an identifier in a `type:` field, or
; the head of a struct initialiser. The wildcard covers every declaration that
; names one without listing them.
(_ type: (identifier) @type)
(struct_initializer . (identifier) @type)

; File-local names. A declaration inside a function, a container's field, a
; parameter, and the `|payload|` an `if`, `while`, `for` or `catch` binds
; (ADR 0030). Without these the catch-all below took a declaration for a READ.
(variable_declaration . (identifier) @local_def)
(function_declaration name: (identifier) @local_def)
(parameter name: (identifier) @local_def)
(payload (identifier) @local_def)
(container_field name: (identifier) @local_def)

(identifier) @local_ref
