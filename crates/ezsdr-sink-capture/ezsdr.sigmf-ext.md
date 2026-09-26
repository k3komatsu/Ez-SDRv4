# The `ezsdr` SigMF extension, version 1.0.0

The extension namespace that `ezsdr.sink.capture` 1.1.0 writes into every SigMF Recording's metadata (`design/10-host-data-path.md`, HD-15). It carries what SigMF's `core` namespace has no field for: where the stream lost samples, and which channels were valid (`design/02-stream-contract.md`, SC-32). It is declared `optional`, so a reader that does not support it can still read the Recording.

A **file index** is a sample index in the Dataset file, which holds only the samples the stream delivered. A **global index** is a sample index in the stream's SampleClock, as `core:global_index` uses. Both count multi-channel samples (one index per sample instant, whatever `core:num_channels` is).

## Global object

| Field | Required | Type | Description |
|---|---|---|---|
| `ezsdr:sample_rate` | true | object | The exact sample rate as a rational, `{ "num": integer, "den": integer }`; `core:sample_rate` is its floating-point value. |
| `ezsdr:partial` | true | boolean | Whether the capture stopped before it held the samples it was asked for (a stop, an abort, or a contract change); the Dataset then holds what was recorded. |
| `ezsdr:gaps` | true | array | One object per stream gap, in stream order (below). A gap is not in the Dataset file; the capture segments around it carry the jump in `core:global_index`. |
| `ezsdr:valid` | true | array | One array per channel, in channel order, of `{ "sample_start": file index, "sample_count": integer }`: the runs over which that channel's samples are valid. A sample outside every run of its channel is present in the file but not valid. |

A gap object:

| Field | Type | Description |
|---|---|---|
| `sample_start` | integer | The file index of the first sample after the gap; the number of samples in the file when none follows. |
| `global_index` | integer | The global index of the gap's first missing sample. |
| `len` | integer | The gap's extent in samples. `0` for a loss after the last delivered sample, whose extent is unknown. |
| `lost` | integer or null | What the stream reported losing, when it did. |
| `cause` | object | Why, as the Stream Contract derives it: `{ "kind": "stream" \| "overflow_restart" \| "sequence_error" \| "alignment" \| "link_drop" \| "unknown" }`, or `{ "kind": "mixed", "stream_lost": integer }`. |
| `link_dropped` | integer | How many blocks the host link in front of the recorder discarded inside the gap. |

## Annotation objects

One annotation per interval in which one channel was invalid while the stream continued, with `core:sample_start`, `core:sample_count` and `core:label` `"invalid channel"`, and:

| Field | Type | Description |
|---|---|---|
| `ezsdr:channel` | integer | The channel. |
| `ezsdr:cause` | object | `{ "kind": "alignment" }` when the device reported a multi-channel alignment failure, `{ "kind": "stream" }` otherwise. |

A channel that was never valid in the Recording has no annotation; its `ezsdr:valid` array is empty.
