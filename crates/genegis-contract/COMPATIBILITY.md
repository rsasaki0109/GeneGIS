# GeoContract v0 compatibility truth table

This table is normative for `GeoContract::compatibility_with` in schema
version `0.1.0`.

| Required semantic | Provided semantic | Result |
| --- | --- | --- |
| absent | any | `compatible` — the consumer declared no constraint |
| known value | same known value | `compatible` |
| known value | different known value | `incompatible` |
| known value | absent or unknown | `indeterminate` — never treated as a match |
| required tolerance | equal or stricter maximum | `compatible` |
| required tolerance | weaker maximum | `incompatible` |
| required tolerance | missing metric | `indeterminate` |

The overall result is the worst field result:

```text
incompatible > indeterminate > compatible
```

Compatibility is directional: `provided.compatibility_with(required)`. An
unspecified optional field in the required contract is a wildcard; an
unspecified required domain is not a request to erase the provider's metadata.

Before release admission, both contracts must pass `GeoContract::validate`.
The validator rejects unknown CRS, coordinate units, axis order, measure kind,
aggregation basis, temporal granularity, join uniqueness, and null policy when
their corresponding domain is present. A future `VerificationPolicy` may allow
an exploratory run to retain unknowns, but unknowns cannot yield a verified
compatibility result.
