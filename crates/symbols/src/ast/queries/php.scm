; PHP cannot declare a type inside another one: a `class_declaration` is either
; at file scope or inside a `namespace` block, and both are reached by the same
; fully-qualified name. So there is no file-scope anchor to apply here, exactly
; as there is none in C#.
(class_declaration name: (name) @def)
(interface_declaration name: (name) @def)
(trait_declaration name: (name) @def)
(enum_declaration name: (name) @def)
(function_definition name: (name) @def)
(program (const_declaration (const_element (name) @def)))

; A method a CLASS body declares. An interface's and a trait's are not: every
; implementor and every using class declares that name (ADR 0030), and all three
; bodies are spelled `declaration_list`, so the owner has to be named.
(class_declaration body: (declaration_list (method_declaration name: (name) @def)))
(enum_declaration body: (enum_declaration_list (method_declaration name: (name) @def)))

(function_call_expression function: (name) @call)
(member_call_expression name: (name) @call)
(nullsafe_member_call_expression name: (name) @call)
(scoped_call_expression name: (name) @call)

; Named without being called: `Mode::Fast` reads a case by path and
; `$this->label` a property. PHP spells a qualified name and a member read the
; same way, so this carries both — Go's known edge, in this language.
(member_access_expression name: (name) @ref)
(nullsafe_member_access_expression name: (name) @ref)
(class_constant_access_expression . (name) (name) @ref)

; A type is a bare `name` in a type position, in an inheritance clause, or the
; head of a `new`. PHP has no type-specific node to key on.
(named_type (name) @type)
(object_creation_expression (name) @type)
(class_interface_clause (name) @type)
(base_clause (name) @type)
(class_constant_access_expression . (name) @type)
(scoped_call_expression scope: (name) @type)

; File-local names. PHP declares a variable by assigning to it, so the
; assignment's left side IS the binding position; everything else is a
; parameter, a property, a class constant or an enum case, each reached through
; the thing that owns it (ADR 0030). Without these the catch-all below took a
; declaration for a READ.
(assignment_expression left: (variable_name (name) @local_def))
(list_literal (variable_name (name) @local_def))
(simple_parameter name: (variable_name (name) @local_def))
(variadic_parameter name: (variable_name (name) @local_def))
(property_promotion_parameter name: (variable_name (name) @local_def))
(catch_clause name: (variable_name (name) @local_def))
(foreach_statement (pair (variable_name (name) @local_def)))
(foreach_statement . (_) (variable_name (name) @local_def))
(anonymous_function_use_clause (variable_name (name) @local_def))
(property_element name: (variable_name (name) @local_def))
(const_element (name) @local_def)
(enum_case name: (name) @local_def)
(method_declaration name: (name) @local_def)

(name) @local_ref
