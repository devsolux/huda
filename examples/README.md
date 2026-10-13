# Explicit test inputs

`manual_gold.py` is an authored native client E2E runner. It mutates only the explicitly selected Huda profile and requires a successfully compiled helper plus previously initialized credentials. Use a disposable private test profile. It does not grant provider/model consent. No successful run has been recorded yet.

`entity.json` works with `huda entity-id`. `empty.json` is an empty request for read-only handlers. `local-model.example.json` is intentionally **invalid until real model identity, digest and license are supplied**. It must not silently activate or download a model.
