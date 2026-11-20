# Bundled fonts

Inter and JetBrains Mono use the SIL Open Font License 1.1; Phosphor uses the MIT license.
The license files sit beside the fonts and ship as package resources.

| File | From | SHA-256 |
|---|---|---|
| `Inter-Regular.ttf` | [Inter 4.1](https://github.com/rsms/inter/releases/tag/v4.1), `extras/ttf/` | `40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82` |
| `Inter-SemiBold.ttf` | Inter 4.1, `extras/ttf/` | `78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3` |
| `JetBrainsMono-Regular.ttf` | [JetBrains Mono 2.304](https://github.com/JetBrains/JetBrainsMono/releases/tag/v2.304), `fonts/ttf/` | `a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f` |
| `Phosphor.ttf` | [@phosphor-icons/web 2.1.2](https://registry.npmjs.org/@phosphor-icons/web/-/web-2.1.2.tgz), `src/regular/Phosphor.ttf` (Regular) | `06b91e022b7ee899a63efced879392a74f0bacbda54e4467e9f663220d173a10` |

`theme::fonts` embeds them; `theme.rs` tests check that they cover every character the
interface writes and that every `Icon` codepoint exists in Phosphor's cmap. The icon
mapping in `icons.rs` comes from the same package's `src/regular/style.css`.
Phosphor uses a dedicated `icons` family because Inter also maps some of its private-use
codepoints. Inline icons belong in separate text sections using that family.
CPU atlas tests compare every icon's pixels and metrics with a Phosphor-only reference,
including after both Inter weights have populated the shared atlas.
