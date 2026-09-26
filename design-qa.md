# Auralis GUI design QA

## Evidence

- Source visual truth: private local reference image (1487 x 1058 px; not included in the repository).
- Rendered implementation: historical `/tmp/auralis-gui-balanced-active-final.png` capture (1440 x 1024 px, CSS viewport 1440 x 1024, device scale factor 1; removed after QA cleanup).
- Normalized comparison input: historical `/tmp/auralis-gui-design-compare-final.png` capture (source and implementation side by side; implementation resized to 1487 x 1058 for comparison only; removed after QA cleanup).
- State: Console, Balanced, running, local model path, active processing state.
- Environment: Chrome via Playwright CLI against `http://127.0.0.1:8766`; audio API exercised in WSL/PulseAudio, not native Windows WASAPI.

## Comparison

The combined full-view comparison preserves the source's dark left navigation,
quiet white workspace, teal action treatment, thin rules, restrained typography,
and diagnostics-first information hierarchy. The implementation adds the model
path field and explicit runtime diagnostics required by Auralis; those controls
make the local denoiser path usable and are intentionally denser than the
visual target.

Focused checks covered the action/profile row, diagnostics grid, profile
visibility, and the 390 px responsive layout. The diagnostics labels and
values remain paired after the grid fix. At 390 px, `scrollWidth` equals
`innerWidth` (390 px), so no horizontal overflow hides controls.

## Fidelity surfaces

- Fonts and typography: Segoe UI/system fallback, medium display weight, clear
  label/value hierarchy, tabular numeric diagnostics, and ellipsis for long
  runtime values. The implementation heading is slightly heavier than the
  source to preserve readability at the actual desktop viewport; this is a
  P3 polish difference.
- Spacing and layout rhythm: 240 px navigation rail, generous desktop content
  margins, 46 px controls, 154 px primary action, and consistent rule spacing.
  The implementation uses a two-column action/control arrangement and a model
  path row rather than the source's centered action-only composition; this is an
  intentional product constraint, not a broken state.
- Colors and tokens: slate navigation, white workspace, muted slate text,
  teal start state, and semantic red stop state map cleanly to the source
  direction. Focus rings use a visible teal outline.
- Image quality and assets: no remote or raster assets are required by the
  implementation. The source's small navigation glyphs have no supplied asset
  files; text-first navigation was retained to avoid inventing an icon set.
- Copy and content: the screen explicitly says processing is local and keeps
  model latency, software-pipeline latency, and physical end-to-end latency
  separate. No physical latency claim is inferred.

## Comparison history

1. Initial pass: the technical `<dl>` placed labels and values in different
   columns, and Chrome reported a missing favicon. The grid was changed to
   paired label/value items and a data favicon was added.
2. Second pass: profile selection was overwritten by the 500 ms state poll,
   Low latency showed an irrelevant model field, and the idle diagnostics kept
   the previous profile's engine preview. Initial-only form synchronization,
   profile-specific field visibility, and local profile preview metadata fixed
   these issues.
3. Final pass: the Balanced active screen was recaptured at the same CSS
   viewport, the combined comparison input was reviewed, Console/Settings/About
   navigation and Start/Stop were exercised, profile switching and path
   retention were rechecked, and the browser console reported 0 errors and 0
   warnings.

## Final result

No actionable P0/P1/P2 visual or interaction findings remain. Native Windows
visual/device verification remains a separate acceptance task.

final result: passed
