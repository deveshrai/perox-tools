; Slint highlighting for PeroxIDE (tree-sitter-slint ships no query).
; Later patterns win over earlier ones for the same node.

(comment) @comment
(string_value) @string
(escape_sequence) @escape
[
  (int_value) (float_value) (length_value) (physical_length_value) (duration_value)
  (angle_value) (percent_value) (relative_font_size_value)
] @constant.builtin
(color_value) @constant
(bool_value) @constant.builtin

(builtin_type_identifier) @type.builtin
(user_type_identifier) @type

(component_definition name: (user_type_identifier) @type)
(component id: (simple_identifier) @variable.parameter)
(component type: (user_type_identifier) @type)

(property name: (simple_identifier) @property)
(binding name: (simple_identifier) @property)
(property_assignment property: (simple_identifier) @property)
(callback name: (simple_identifier) @function)
(callback_event name: (simple_identifier) @function.method)
(function_definition name: (simple_identifier) @function)
(function_call name: (simple_identifier) @function)
(typed_identifier name: (simple_identifier) @variable.parameter)

(property_visibility) @keyword
(function_visibility) @keyword
(purity) @keyword

[
  "import" "from" "component" "inherits" "property" "callback" "struct" "enum"
  "global" "for" "in" "out" "in-out" "private" "public" "pure" "if" "else" "return"
  "animate" "states" "transitions" "when" "function" "changed" "as"
] @keyword

[ "@image-url" "@linear-gradient" "@radial-gradient" "@tr" ] @function.macro

[ "root" "parent" "self" ] @variable.builtin
[ "true" "false" ] @constant.builtin

[ "{" "}" "(" ")" "[" "]" ] @punctuation.bracket
[ ";" "," "." ":" ] @punctuation.delimiter
[ "=>" "<=>" ":=" ] @operator
