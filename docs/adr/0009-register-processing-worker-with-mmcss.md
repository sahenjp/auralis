# ADR 0009: Register the Windows processing worker with MMCSS

- Status: accepted after native scheduler-starvation evidence
- Date: 2026-08-26

## Problem

A corrected 10-minute Windows run recorded one 480-sample steady-state
underrun at 324.66 seconds. The output queue was empty while the input queue was
at its four-frame limit. Capture and render callbacks continued, but the
ordinary-priority processing worker did not run for roughly four callback
periods. Increasing queue capacity would add latency and hide the scheduling
failure.

## Decision

On Windows, register only the processing worker with MMCSS task `Pro Audio` at
relative priority `normal`. Perform registration once on the worker, outside
audio callbacks. Keep the handle in thread-local storage so it is reverted by
`AvRevertMmThreadCharacteristics` on the same thread at exit. Report the
requested task/priority, attempt, success, and error in every native run.

Do not raise process-wide priority, busy-spin the worker, change MMCSS registry
configuration, or add Windows scheduling types to `auralis-core`.

## Consequences

The platform call stays in `auralis-wasapi`, while the processing contract
remains OS-independent. Registration adds a one-time startup operation to the
first worker frame; it added no processing deadline miss in accepted runs.
The accepted 60-second, 10-minute, and 30-minute runs registered successfully
and each recorded zero steady-state underrun, overrun, xrun, or stream error.

If MMCSS registration fails, the stream still runs and the machine report
contains the error; that run is not accepted as the Windows stability baseline.

## References

- Microsoft, [Multimedia Class Scheduler Service](https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service)
- Microsoft, [`AvSetMmThreadPriority`](https://learn.microsoft.com/en-us/windows/win32/api/avrt/nf-avrt-avsetmmthreadpriority)
- Microsoft, [Exclusive-mode stream scheduling example](https://learn.microsoft.com/en-us/windows/win32/coreaudio/exclusive-mode-streams)
