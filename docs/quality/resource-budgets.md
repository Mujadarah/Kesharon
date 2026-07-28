# Resource Budgets

These ceilings are product requirements. Tests may use tighter platform-specific
warning thresholds, but releases may not silently weaken the ceilings.

| Metric | Ceiling |
| --- | --- |
| Windows/macOS installer | 50 MB |
| Linux installer | 40 MB |
| Base installed footprint | 120 MB |
| Cold idle RAM | 220 MB |
| Typical active RAM, excluding spawned tools | 650 MB |
| Idle CPU over five minutes | 1% of one core |
| Idle disk writes after warm-up | 1 MB/hour |
| Default indexer workers | 2 |
| Diagnostic logs | 20 MB |
| Temporary artifacts | 1 GB |

CI records measurements from native runners. Resource regressions fail once
stable platform baselines exist; until then, missing measurements fail the
milestone gate rather than being treated as a pass.
