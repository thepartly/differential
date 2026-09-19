; Definitions. A C# type is reached by name from anywhere — a nested one as
; `Outer.Inner`, a namespaced one by its `using` — so unlike TypeScript there is
; no file-scope anchor to apply and no `export` to gate on.
(class_declaration name: (identifier) @def)
(interface_declaration name: (identifier) @def)
(struct_declaration name: (identifier) @def)
(record_declaration name: (identifier) @def)
(enum_declaration name: (identifier) @def)
(delegate_declaration name: (identifier) @def)

; A method a class, struct or record body declares. An interface's method is
; not one: every implementor declares that name (ADR 0030). C# spells both
; bodies `declaration_list`, so the owner has to be named to tell them apart.
(class_declaration body: (declaration_list (method_declaration name: (identifier) @def)))
(struct_declaration body: (declaration_list (method_declaration name: (identifier) @def)))
(record_declaration body: (declaration_list (method_declaration name: (identifier) @def)))

(invocation_expression function: (identifier) @call)
(invocation_expression function: (member_access_expression name: (identifier) @call))

; Named without being called: `Config.Timeout` reads a constant by path. C#
; spells a qualified name and a member read the same way, so this carries field
; reads too — Go's known edge, in this language.
(member_access_expression name: (identifier) @ref)

; C# has no `type_identifier` node: a type is a plain identifier sitting in a
; `type:` field, or the head of a `generic_name`. The wildcard is what covers
; every declaration that names one — a parameter, a variable, a `new`, an `is`
; pattern, a `catch` — without listing them.
(_ type: (identifier) @type)
(_ type: (generic_name (identifier) @type))
(_ type: (qualified_name name: (identifier) @type))
(method_declaration returns: (identifier) @type)
(method_declaration returns: (generic_name (identifier) @type))
(base_list (identifier) @type)
(base_list (generic_name (identifier) @type))

; File-local names. An interface's method, a property, a field, a local and
; every binding position: each reaches only its own file, so it may only draw an
; edge inside it (ADR 0030). Without these the catch-all below took them as
; READS.
(method_declaration name: (identifier) @local_def)
(property_declaration name: (identifier) @local_def)
(variable_declarator name: (identifier) @local_def)
(parameter name: (identifier) @local_def)
(implicit_parameter) @local_def
(foreach_statement left: (identifier) @local_def)
(catch_declaration name: (identifier) @local_def)
(declaration_pattern name: (identifier) @local_def)
(declaration_expression name: (identifier) @local_def)
(enum_member_declaration name: (identifier) @local_def)
(type_parameter name: (identifier) @local_def)

(identifier) @local_ref
