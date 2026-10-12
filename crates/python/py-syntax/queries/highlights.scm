; Highlight query of Pyoxide for Python (tree-sitter-python grammar).
;
; Capture names map to theme text roles (docs/theme-mapping.md). For the
; same node the first pattern in this file wins; an inner node wins over the
; node around it. Captures starting with `_` are not drawn but still win for
; their node. Names are coloured the way the reference IDE colours Python by
; default: plain names, parameters and attributes keep the text colour.

; ---- strings and comments -------------------------------------------------

; Docstrings: a string that is the first statement of a module, class or
; function.
(module . (expression_statement (string) @string.documentation))
(class_definition
  body: (block . (expression_statement (string) @string.documentation)))
(function_definition
  body: (block . (expression_statement (string) @string.documentation)))

(string) @string
(escape_sequence) @string.escape

; Inside f-strings the expressions are code again.
(interpolation
  "{" @punctuation.special
  expression: (_) @embedded
  "}" @punctuation.special)

((comment) @comment.todo
  (#match? @comment.todo "(TODO|FIXME|todo|fixme)"))
(comment) @comment

; ---- literals -------------------------------------------------------------

[
  (integer)
  (float)
] @number

[
  (true)
  (false)
  (none)
  (ellipsis)
] @constant.builtin

; ---- decorators and definitions -----------------------------------------

(decorator) @function.decorator

; The decorator's name keeps the decorator colour (not a call or built-in).
(decorator
  (identifier) @_decorator_name)
(decorator
  (call
    function: (identifier) @_decorator_name))
(decorator
  (call
    function: (attribute
      attribute: (identifier) @_decorator_name)))

((function_definition
  name: (identifier) @function.dunder)
  (#match? @function.dunder "^__[A-Za-z0-9_]+__$"))

(class_definition
  body: (block
    (function_definition
      name: (identifier) @function.method)))
(class_definition
  body: (block
    (decorated_definition
      definition: (function_definition
        name: (identifier) @function.method))))

(function_definition
  name: (identifier) @function)

(class_definition
  name: (identifier) @type.definition)

; ---- names --------------------------------------------------------------

(keyword_argument
  name: (identifier) @keyword.argument)

((identifier) @variable.self
  (#any-of? @variable.self "self" "cls"))

((identifier) @variable.dunder
  (#match? @variable.dunder "^__[A-Za-z0-9_]+__$"))

; Called methods; other attribute names stay plain (and are never built-ins).
(call
  function: (attribute
    attribute: (identifier) @function.method.call))
(attribute
  attribute: (identifier) @_attribute)

((identifier) @function.builtin
  (#any-of? @function.builtin
    "abs" "aiter" "all" "anext" "any" "ascii" "bin" "bool" "breakpoint"
    "bytearray" "bytes" "callable" "chr" "classmethod" "compile" "complex"
    "delattr" "dict" "dir" "divmod" "enumerate" "eval" "exec" "filter"
    "float" "format" "frozenset" "getattr" "globals" "hasattr" "hash" "help"
    "hex" "id" "input" "int" "isinstance" "issubclass" "iter" "len" "list"
    "locals" "map" "max" "memoryview" "min" "next" "object" "oct" "open"
    "ord" "pow" "print" "property" "range" "repr" "reversed" "round" "set"
    "setattr" "slice" "sorted" "staticmethod" "str" "sum" "super" "tuple"
    "type" "vars" "zip" "__import__"
    "BaseException" "Exception" "ArithmeticError" "AssertionError"
    "AttributeError" "EOFError" "ImportError" "IndexError" "KeyError"
    "KeyboardInterrupt" "LookupError" "MemoryError" "NameError"
    "NotImplementedError" "OSError" "OverflowError" "RecursionError"
    "RuntimeError" "StopIteration" "StopAsyncIteration" "SyntaxError"
    "SystemExit" "TypeError" "ValueError" "ZeroDivisionError"
    "FileNotFoundError" "PermissionError" "TimeoutError" "NotImplemented"))

(call
  function: (identifier) @function.call)

; ---- keywords -------------------------------------------------------------

[
  "and"
  "as"
  "assert"
  "async"
  "await"
  "break"
  "case"
  "class"
  "continue"
  "def"
  "del"
  "elif"
  "else"
  "except"
  "finally"
  "for"
  "from"
  "global"
  "if"
  "import"
  "in"
  "is"
  "is not"
  "lambda"
  "match"
  "nonlocal"
  "not"
  "not in"
  "or"
  "pass"
  "raise"
  "return"
  "try"
  "type"
  "while"
  "with"
  "yield"
] @keyword
