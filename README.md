# Scenario Runner

Run or validate exactly one binary scenario inside a sandbox and emit an
evidence bundle. The runner accepts LM-proposed scenarios but still does not infer
semantics, mutate inputs, or retry. The binary behavior is the oracle.

## Usage

```
bman ls
```

Optional flags:

```
bman ls --dry-run
bman ls --out-dir ./out
bman ls --direct
bman ls --verbose
```

`--dry-run` validates without execution. `--direct` skips bwrap and is intended
for debugging only. `--verbose` prints a workflow transcript (including LM
prompt/response and scenario JSON) to stderr.

The positional argument is the target binary name or path. `bman` invokes the
embedded LM CLI to generate the scenario JSON and requires the LM tool to be
authenticated/configured with network access.

LM command configuration:
- Default command is Claude CLI (`claude`).
- Override with `BMAN_LM_COMMAND` as JSON, for example:
  - `export BMAN_LM_COMMAND='{"command":["claude","--print","--output-format","json","--json-schema","{schema}","--no-session-persistence","--tools","","{prompt}"]}'`
  - `export BMAN_LM_COMMAND='{"command":["/path/to/other-llm","--json","{prompt}"]}'`
- Placeholders: `{prompt}` is replaced with the full prompt, `{schema}` with `schema/scenario.lm.json`.
- If the command omits `{prompt}`, `bman` writes the prompt to stdin.

For local development:

```
cargo run --bin bman -- ls
```

## Scenario JSON

Example (`scenario.json`):

```json
{
  "scenario_id": "ls_help_smoke",
  "rationale": "Capture basic usage and flags from ls --help.",
  "binary": { "path": "/nix/store/.../bin/ls" },
  "args": ["--help"],
  "fixture": { "id": "fs/empty_dir" },
  "limits": {
    "wall_time_ms": 200,
    "cpu_time_ms": 100,
    "memory_kb": 65536,
    "file_size_kb": 1024
  },
  "artifacts": {
    "capture_stdout": true,
    "capture_stderr": true,
    "capture_exit_code": true
  }
}
```

Notes:
- `args` is an array of strings only (no shell parsing).
- `rationale` is a short, plain-text reason for the scenario.
- `binary.path` must match the target binary path exactly and be executable (symlinks are resolved before hashing).
- `fixture.id` maps to `fixtures/<id>/`.
- Limits are required and bounded in code.
- The scenario JSON is produced by the LM and must match the target binary.

Canonical schema:
- `schema/scenario.v0.json` mirrors the runtime validation rules.
- Unknown fields are rejected.

LM schema:
- `schema/scenario.lm.json` is a minimal JSON Schema used for LM structured output.

Validation is fail-closed. Responses are rejected when:
- JSON fails to parse or includes unknown fields.
- Limits are missing or out of bounds, args contain NUL, or arg counts exceed bounds.
- `fixture.id` is invalid or not in `fixtures/catalog.json`.
- The binary path is missing, not executable, or does not match the target binary.
- The fixture manifest or tree fails verification.

## Fixtures

Fixtures live under `fixtures/`:

```
fixtures/
  fs/
    empty_dir/
      manifest.json
      tree/
```

Fixture catalog:
- `fixtures/catalog.json` lists allowed fixture IDs and descriptions.
- Scenario `fixture.id` must appear in the catalog.

`manifest.json` is authoritative. The runner copies `tree/` into a temp dir,
applies permissions and mtimes from the manifest, and verifies file hashes.

## Examples

```
scenarios/examples/ls_help.json
```

The example is used as a format reference when constructing LM prompts.

## LM interface

Inputs to the model:
- Raw `--help` text for the target binary.
- `fixtures/catalog.json` and the binary path to use.
- `schema/scenario.v0.json`.
- Example scenario JSON (format reference).

Output:
- A single scenario JSON object that conforms to the schema.
- No retries or inference; responses are treated as untrusted input.
- The configured LM CLI must emit raw JSON to stdout.
  - If the LM returns a JSON envelope with `structured_output`, `bman` uses that object.

## LM provenance

The raw LM prompt and response bytes are stored alongside the evidence bundle:

```
lm.prompt.txt
lm.response.json
```

The response is not parsed or modified before it is saved.

## Evidence bundle

Each run writes to `out/evidence/<run_id>/` (or `<out-dir>/evidence/<run_id>/`). The
`run_id` is formatted as `<label>-<hash12>-<epoch_ms>`, where `label` is a
slugged `scenario_id` when available (or an error code for early failures):

```
out/evidence/<run_id>/
  scenario.json
  meta.json
  lm.prompt.txt
  lm.response.json
  stdout.txt   (when captured)
  stderr.txt   (when captured)
```

`meta.json` includes hashes for the binary, scenario, fixture manifest, and
stdout/stderr, plus exit code and timing.

`--dry-run` writes an evidence bundle without executing. The outcome is
`schema_invalid` on validation failure or `exited` when the response is valid.

## Environment contract

All runs set:
- `LC_ALL=C`
- `TZ=UTC`
- `TERM=dumb`

stdin is always `/dev/null`. Network is disabled inside the sandbox.
