# ADR 0005: Retain CPAL with a narrow WASAPI period probe

- Status: accepted
- Date: 2026-08-26

## Problem

Milestone 2 needs native Windows endpoint identity, formats, callback timing,
stream clocks, xruns, actual buffer sizes, and `IAudioClient3` shared-engine
period information. Replacing CPAL would add a second stream lifecycle and a
large unsafe COM surface.

## Evidence

CPAL 0.18.2 exposes stable device IDs, input and output callback timestamps,
`Stream::now`, `Stream::buffer_size`, and xrun-classified stream errors. Its
Windows device wrapper exposes the underlying `IMMDevice`. Source inspection
also shows that CPAL initializes the shared stream through `IAudioClient`, not
`IAudioClient3::InitializeSharedAudioStream`.

Microsoft documents `GetSharedModeEnginePeriod` as the query for default,
fundamental, minimum, and maximum periods for a format. It documents
`GetCurrentSharedModeEnginePeriod` as the query for the current format and
period.

## Decision

Keep CPAL as the only audio stream owner. Add a Windows-only, read-only
`auralis-wasapi` adapter that obtains CPAL's `IMMDevice`, activates
`IAudioClient3`, and records the two period queries. Keep this adapter out of
`auralis-core`.

Do not add a native WASAPI transport backend now. Revisit only if a required
timestamp, clock, period-control, recovery, or callback behavior cannot be
obtained or controlled through CPAL.

## Consequences

The realtime callbacks keep the same small safe-Rust surface. COM ownership and
unsafe code are isolated in one platform crate. Auralis can characterize the
current Windows engine period, but it cannot claim to have requested an
`IAudioClient3` period through CPAL.

## References

- Microsoft, [`IAudioClient3::GetSharedModeEnginePeriod`](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient3-getsharedmodeengineperiod)
- Microsoft, [`IAudioClient3::GetCurrentSharedModeEnginePeriod`](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient3-getcurrentsharedmodeengineperiod)
- Microsoft, [`IAudioClient3::InitializeSharedAudioStream`](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient3-initializesharedaudiostream)
