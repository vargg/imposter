# Bundled fonts

Both fonts use the SIL Open Font License 1.1; the license files sit beside them.

| File | From | SHA-256 |
|---|---|---|
| `Inter-Regular.ttf` | [Inter 4.1](https://github.com/rsms/inter/releases/tag/v4.1), `extras/ttf/` | `40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82` |
| `Inter-SemiBold.ttf` | Inter 4.1, `extras/ttf/` | `78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3` |
| `JetBrainsMono-Regular.ttf` | [JetBrains Mono 2.304](https://github.com/JetBrains/JetBrainsMono/releases/tag/v2.304), `fonts/ttf/` | `a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f` |

`theme::fonts` embeds them; `theme.rs` tests check that they cover every character the
interface writes.
