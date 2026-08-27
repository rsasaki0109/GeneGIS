# GeneGIS Desktop Bridge

This provider-neutral plugin core sends only layers explicitly selected by a
desktop host to the local GeneGIS workbench. It computes SHA-256 over each
exported asset before transport; the Rust host re-hashes the bytes, validates
CRS/unit/license/format contracts, and creates a Command + Workflow-bound
bridge capsule.

Host plugins should call `GeneGISDesktopBridgePlugin.push_selected_layers` from
their selected-layer action. They must export supported open formats first and
must not pass host project files, credentials, executable styles, expressions,
or plugin binaries.

For a standalone smoke run:

```powershell
python integrations/desktop-gis/genegis_bridge_plugin.py request.json
```

The local workbench writes accepted capsules under
`.genegis/bridge-capsules/`; the client cannot select an arbitrary output path.
