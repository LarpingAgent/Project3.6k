# Project3.6k — Phase 2 Verification & Closure Review

- **Review date:** 2026-11-12
- **Code reviewed:** commit `3ebac6d` (`feat(engine): add lifecycle event and timestep core`)
- **Result:** Existing Phase 2 implementation verified; no source or renderer files changed during this review.
- **Recommendation:** Ready for explicit Phase 2 closure confirmation. Phase 3 has not started.

## 1. Fixed and variable timestep

### Exact policy

`EngineRuntime::run_frame` uses `core::FrameClock` to measure raw monotonic `frame_time.delta`, then:

1. Accepts `variable_delta = min(frame_time.delta, max_frame_delta)`. The default cap is **100 ms**. Any raw excess is deliberately not passed to either the fixed-step accumulator or the variable-update callback.
2. Adds the accepted delta to the fixed accumulator.
3. Calls fixed update at **16,666,667 ns** intervals by default, with at most **5 fixed updates per frame** by default. Configuration validation caps this at **32**.
4. If whole fixed intervals remain after the update cap, discards those whole intervals, keeps only the fractional remainder for interpolation, logs the discarded amount, and returns it as `FrameSchedule::dropped_simulation_time`.
5. Calls variable update once with the accepted `variable_delta` and the remaining accumulator fraction as interpolation alpha. Platform renders after `run_frame` returns.
6. An exit event is dispatched before updates; the frame skips fixed and variable updates. Shutdown clears any residual accumulator.

The fixed-step catch-up drop is intentional to bound work and avoid a spiral of death. The frame-delta cap is also an intentional stability policy: raw time above 100 ms is omitted from updates. That clamp loss is **not** included in `dropped_simulation_time`; it is recoverable per frame from `frame_time.delta - variable_delta`. The field reports only whole accumulator steps discarded after the fixed-update cap.

### Evidence

- `cargo test --workspace` executed the existing deterministic tests `fixed_updates_are_capped_and_excess_time_is_discarded_deterministically` and `variable_delta_is_clamped_and_fixed_tick_ids_increase`; both passed.
- The frame-spike test feeds a 250 ms raw delta with a 100 ms cap and confirms the variable callback receives 100 ms and fixed work is capped at four test-configured ticks. With its 10 ms fixed interval, the schedule advances 40 ms, drops 60 ms of accepted accumulator time after the four-tick cap, and omits the 150 ms raw excess: **210 ms total is omitted from simulation for that frame**. The existing test asserts the accepted delta and fixed-tick cap, but does not directly assert the dropped-time field or total.
- Release resize/minimize/restore stress reached a **1,556.602 ms maximum** observed redraw interval in the 120-sample profile window (19.57 FPS window average, 51.10 ms mean interval, 38.61 ms p95). Three catch-up warnings each reported about **16.666667 ms** dropped by the accumulator policy. The raw clamp portion is visible from the raw profiler interval and accepted-delta cap, but it is not separately logged.
- The resize stress completed 180 submitted frames, observed **8 resize events** and **1 zero-extent event**, restored the window, and exited with code 0 and clean Vulkan validation.

The long resize frames are caused by the existing Platform/Vulkan path rebuilding the reference scene and reimporting/reuploading assets during swapchain recreation. This review did not change that behavior.

## 2. Event queue and input transitions

- Default event queue capacity is **2,048**; validated configuration permits at most **65,536**. A full queue returns `EngineError::EventQueueFull` before changing input state or enqueueing the rejected event.
- Accepted events are dispatched FIFO. Input state is updated when an event is enqueued, so it represents the most recently accepted input state by the next update; event callbacks are drained in FIFO order at `run_frame`.
- Focus loss clears all logical actions and pointer delta when the focus event is accepted. Existing tests cover queue overflow leaving the rejected action inactive, FIFO dispatch, action press/release, pointer accumulation, focus-loss clearing, and independent left/right Shift release.
- Critical OS event paths were inspected: `CloseRequested` calls `event_loop.exit()` even if queue insertion fails. If focus-loss or an input-release event cannot be queued, Platform records an application failure and exits; `EngineApplication::drop` requests shutdown, `EngineRuntime::shutdown` clears input/events, and cursor capture is released. Thus overflow fails closed instead of continuing with a stuck held action.
- The overflow tests cover the Engine queue and state update. There is no automated test that forces a full queue through a live winit window while injecting close/focus/key-release events; those Platform overflow branches are verified by code inspection, not by an executed OS-level stress test.

## 3. Lifecycle, cleanup, and ownership

- `EngineRuntime` transitions `Created → Running → StopRequested → Stopped`; shutdown is idempotent. A stopped runtime is intentionally one-shot: starting the same instance again returns an invalid-transition error. Each application launch creates a fresh runtime.
- `cargo test --workspace` passed the lifecycle transition test and `tests/lifecycle_flow.rs`, which checks start → event → fixed update → variable update → render → shutdown ordering.
- Release `--run-frames 5 --validation` initialized the renderer and engine, submitted five frames, shut down, and exited 0 with **0 Vulkan validation warnings/errors**.
- Release `--auto-close --validation` was run **three times**. All three separate application/runtime instances started and stopped with exit code 0 and **0 Vulkan validation warnings/errors**.
- `EngineRuntime` contains no Window, Vulkan, or renderer fields and its crate has no `winit`, `ash`, Platform, or Vulkan dependency. It uses an `Rc` phantom marker to prevent `Send`/`Sync` use across threads.
- `EngineApplication` owns the runtime, Window, and renderer. It stops Engine, releases cursor capture, drops Vulkan renderer/surface, then drops the Window. Vulkan validation and RAII behavior were clean in the executed runs. No process-wide heap leak detector was run; the evidence is lifecycle teardown plus Vulkan validation, not a proof about all process allocations.

## 4. Architecture compliance

`cargo metadata --no-deps` workspace-member traversal confirmed this acyclic graph:

```text
app      → platform
platform → core, engine, vulkan
engine   → core
vulkan   → core, assets
core     → (no workspace crates)
assets   → (no workspace crates)
```

The Engine crate has no Platform/Vulkan coupling. The existing `platform → vulkan` edge remains, as documented, because renderer/backend abstraction belongs to a later phase. Phase 2 reuses `core::FrameClock`, keeps `FrameProfiler` in Platform, uses the app's existing `tracing_subscriber`, and keeps the current `VulkanRenderer`. No Bevy or other complete engine dependency is present.

## 5. Commands executed and results

| Command / run | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `cargo test --workspace` | **61 passed, 0 failed** |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo build --release -p project3-6k-app` | Passed |
| `cargo tree --workspace -e normal` | Inspected actual workspace dependency tree |
| `cargo metadata --no-deps --format-version 1` piped into PowerShell workspace-member graph traversal (script below) | Acyclic graph shown above |
| `$env:RUST_LOG = "info"; .\target\release\project3-6k.exe --run-frames 180 --resize-stress --validation` | Exit 0; 180 frames; 8 resize events; 1 zero-extent event; restored; 0 validation warnings/errors |
| `$env:RUST_LOG = "debug"; .\target\release\project3-6k.exe --run-frames 180 --resize-stress --validation` | Exit 0; profiler window: 19.57 FPS average, 51.10 ms mean, 38.61 ms p95, 1,556.602 ms maximum interval; 0 validation warnings/errors |
| `target\release\project3-6k.exe --run-frames 5 --validation` | Exit 0; 5 frames; 0 validation warnings/errors |
| `target\release\project3-6k.exe --auto-close --validation` × 3 | All three exit 0; clean Engine/Vulkan shutdown; 0 validation warnings/errors each |

The exact PowerShell pipeline used to derive the workspace-only graph and detect cycles was:

```powershell
$meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$workspace = @{}
foreach ($pkg in $meta.packages) { if ($meta.workspace_members -contains $pkg.id) { $workspace[$pkg.name] = $pkg } }
$edges = @{}
foreach ($pkg in $workspace.Values) {
    $edges[$pkg.name] = @($pkg.dependencies | ForEach-Object { $target = $_.name; if ($_.package) { $target = $_.package }; if ($workspace.ContainsKey($target)) { $target } } | Sort-Object -Unique)
}
$state = @{}
$visit = $null
$visit = { param($node) if ($state[$node] -eq 1) { throw "Cycle detected at $node" }; if ($state[$node] -eq 2) { return }; $state[$node] = 1; foreach ($child in $edges[$node]) { & $visit $child }; $state[$node] = 2 }
foreach ($node in $edges.Keys) { & $visit $node }
foreach ($node in ($edges.Keys | Sort-Object)) { "$node -> $($edges[$node] -join ', ')" }
'Cycle check: acyclic'
```

## 6. Confirmed defects and unresolved risks

**No functional Phase 2 defect was found in the exercised lifecycle, event, or timestep behavior.** Time discarded at the frame-delta cap and at the fixed-update catch-up cap is intentional and bounded. The raw delta and accepted delta are both present in `FrameSchedule`, so the clamp amount is recoverable; the current warning and `dropped_simulation_time` field report only accumulator catch-up drops. This distinction was previously under-documented and is now stated explicitly.

Unresolved verification limits:

- Live OS-level queue saturation followed by close/focus/key-release was not forced; those error/shutdown paths were verified by inspection.
- Repeated startup/shutdown was tested across three separate application instances, not by restarting one `EngineRuntime`; the same instance is intentionally non-restartable.
- Validation/RAII runs are not a process-wide memory leak measurement.
- Resize/minimize causes large frame spikes due to existing renderer scene rebuild/re-upload; the worst observed interval was 1.557 s. Renderer behavior is out of this review's scope.
- Frame-delta clamp time is derivable but not separately logged. A separate clamp metric/log could improve diagnostics later; it is not required to change the current scheduling behavior.

## 7. Phase 2 closure recommendation

Recommend closing Phase 2 after the user explicitly confirms this verification. The current implementation satisfies the reviewed Engine Core scope; the exact intentional time-drop policy and the remaining verification limits are recorded here. **Do not start Phase 3 until that explicit closure confirmation is received.**
