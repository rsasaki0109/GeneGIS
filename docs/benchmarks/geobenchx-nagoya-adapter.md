# GeoBenchX-derived Nagoya strict-artifact adapter

This adapter reuses one task from the MIT-licensed
[GeoBenchX repository](https://github.com/solirinai/geobenchx) at commit
`bb3cd88f6834dee8004a2add6f9f0c150053788d`:

- ID: `TASK_250309_135125_908870`
- prompt: “Visualize total population distribution by country.”
- upstream intent: load population and boundaries, merge them, and produce a
  choropleth.

The task is geographically recast to the GeneGIS north-star Nagoya ward
population-density workflow. This is an adapter/scorer conformance slice, not
an official or comparable GeoBenchX leaderboard score.

`genegis bench external --json` executes the real Command + Workflow Graph and
scores the typed spatial artifact independently. Ward keys and populations are
exact; CRS and units must match; area and density allow 5,000 ppm; geometry is
exact; row ordering is ignored. The seven fixtures contain three accepted
outputs and four deliberate failures: missing row, out-of-tolerance density,
changed geometry, and wrong CRS. The report records runner/scorer identities,
candidate digests, failed predicates, timing, upstream revision, and license.

Copyright notice for reused GeoBenchX task metadata: Copyright (c) 2025
Varvara Krechetova, MIT License.
