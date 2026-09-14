; Definitions: EXPORTED, at the top level. A definition is a name others can
; use, and in a module system `export` is exactly that predicate — so the same
; gate applies to a type as to a value. It did not, once: `type FormData = …`
; unexported in a test file was the only thing in one change that "defined"
; that name, and every file mentioning it linked to the test.
(program (export_statement declaration: (function_declaration name: (identifier) @def)))
(program (export_statement declaration: (class_declaration name: (type_identifier) @def)))
(program (export_statement declaration: (interface_declaration name: (type_identifier) @def)))
(program (export_statement declaration: (type_alias_declaration name: (type_identifier) @def)))
(program (export_statement declaration: (enum_declaration name: (identifier) @def)))

; An EXPORTED file-scope value declaration, which in this language is where the
; components, the hooks and the constants live: `export const Panel = () => …`
; introduces `Panel` exactly as `function Panel()` would. Leaving these out made
; the graph a TYPE graph — on one corpus range every single edge came from an
; interface name.
;
; Exported, and only exported. A definition is a file-scope name OTHERS CAN USE
; (ADR 0023), and in a module system `export` is exactly that predicate. A bare
; top-level `const send = vi.fn()` in a test file is not one, and counting it
; linked every production file that calls `send` to that test — the same
; false-definition failure ADR 0023 was written about. Unexported, it is still
; picked up below as a file-local name, so it keeps every edge it can honestly
; draw.
(program (export_statement declaration:
  (lexical_declaration (variable_declarator name: (identifier) @def))))
(program (export_statement declaration:
  (variable_declaration (variable_declarator name: (identifier) @def))))

(call_expression function: (identifier) @call)
(call_expression function: (member_expression property: (property_identifier) @call))

(type_identifier) @type

; File-local names. A binding declared inside a function — or any declaration
; the export gate above turned down — reaches only its own file, so it may only
; ever draw an edge to another class in that file, which is what makes counting
; it safe (ADR 0030).
(class_declaration name: (type_identifier) @local_def)
(interface_declaration name: (type_identifier) @local_def)
(type_alias_declaration name: (type_identifier) @local_def)
(enum_declaration name: (identifier) @local_def)
(function_declaration name: (identifier) @local_def)
(variable_declarator name: (identifier) @local_def)
(object_pattern (shorthand_property_identifier_pattern) @local_def)
(required_parameter pattern: (identifier) @local_def)
(optional_parameter pattern: (identifier) @local_def)
(import_specifier name: (identifier) @local_def)
(namespace_import (identifier) @local_def)

; A binding is a declaration wherever it is written, not only after `const`.
; Array destructuring, a renamed object key, a rest element, a default, a catch
; parameter, a `for…in` target, a bare arrow parameter and a default import all
; introduce a name, and the spec already calls them file-local
; (spec/ordering.md). Without these the catch-all below took them as READS, so a
; line declaring a name pointed at whatever else in the file spelled it the same
; way.
(array_pattern (identifier) @local_def)
(pair_pattern value: (identifier) @local_def)
(rest_pattern (identifier) @local_def)
(assignment_pattern left: (identifier) @local_def)
(catch_clause parameter: (identifier) @local_def)
(for_in_statement left: (identifier) @local_def)
(arrow_function parameter: (identifier) @local_def)
(import_clause (identifier) @local_def)

; Every identifier might be reading one of them. `(identifier)` does not match
; a `property_identifier`, so `data.rule` offers `data` and not `rule`.
(identifier) @local_ref
