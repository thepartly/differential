; `pub` is the whole predicate, exactly as `export` is in TypeScript. Zig has no
; type declaration — a type is a `const` bound to a `struct`, `enum`, `union` or
; `opaque` body, and a value is a `const` bound to anything else — so the SHAPE
; of a declaration says nothing about who can reach it. `pub` says all of it: a
; declaration other files can use, which is ADR 0030's definition of a
; definition.
;
; That matters most for the thing shaped like a type and named by the importer.
; `const Helper = @import("helper.zig")` is a private binding, and taking it
; would make every importing file the definer of the name it imported — the
; `mod template;` false definition ADR 0030 measured at 64% of one range's
; edges. It falls to the file-local rule below, where it still draws every edge
; it honestly can inside its own file. `pub const ReExport = @import(…)` is a
; different statement and is taken: that name IS reachable from other files.
;
; The grammar gives the bound name no field — it is the declaration's first
; named child — so the anchor is what keeps `pub const Alias = Other;` from
; defining `Other` as well.
(source_file (variable_declaration "pub" . (identifier) @def))
(source_file (function_declaration "pub" name: (identifier) @def))

; A `pub` function a container's body declares is reached through that
; container's name from other files (ADR 0030). Zig has no interface to exempt:
; a method is a plain function inside a struct, and `pub` is again the whole of
; the question.
(struct_declaration (function_declaration "pub" name: (identifier) @def))
(union_declaration (function_declaration "pub" name: (identifier) @def))
(enum_declaration (function_declaration "pub" name: (identifier) @def))
(opaque_declaration (function_declaration "pub" name: (identifier) @def))

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
