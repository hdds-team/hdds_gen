# hdds_gen - Known Limitations

## Enumerator annotations not supported

The parser handles the traditional IDL enum value syntax (`enum E { A = 0, B = 1 }`) but
not the annotation form on enumerators:

- `@value(N)` - OMG IDL 4.2 section 8.3.1.5: sets a constant value, for example an explicit enumerator value
- `@default_literal` - OMG IDL 4.2 section 8.3.2.3: selects the default enumerator

Both are refused by `hddsgen check` when written on an enumerator.

`@appendable` and `@extensibility(...)` on an enum declaration are accepted.

**Context:** the DDS-XTypes interop test suites use the annotation form:

```idl
@appendable
enum E {
    @value(0)
    VAL0,
    @value(1)
    VAL1,
    @value(2)
    VAL2
};
```

**Workaround:** rewrite the enumerators with the explicit syntax before code generation:

```idl
@appendable
enum E { VAL0 = 0, VAL1 = 1, VAL2 = 2 };
```

**Impact:** a declaration that relies on `@value(N)` cannot be fed to `hddsgen` as is.

**Status:** not implemented.
