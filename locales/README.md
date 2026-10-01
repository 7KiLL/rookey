# rookey's words

One file per language, named by its code: `en.json`, `uk.json`. The settings page, the command line, the pill and the notifications all read from here, and every file in this folder is built in, so adding a language takes one file and no code.

## Adding a language

1. Copy `en.json` to `<code>.json`, using the two-letter code (`pl.json`, `de.json`).
2. Set `_name` to the language's name in that language (`"Polski"`).
3. Translate the values. Leave the keys as they are.
4. Open a pull request titled `feat(i18n): Polish`.

You don't have to translate everything at once: any key you leave out shows in English. Delete the keys you haven't translated rather than leaving English text in them, so the next person can see what's missing.

## Rules the tests check

- Every key must exist in `en.json`.
- Keep the `{slots}` as written: `{chord}`, `{path}` and the rest are filled in by rookey. You can move them within the sentence, but you can't rename them or add new ones.
- A sentence with a number has one text per plural form, named as in [Intl.PluralRules](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl/PluralRules): English uses `one` and `other`, Ukrainian `one`, `few`, `many` and `other`. Include the forms your language uses.

## Trying it

```sh
cargo build --release
ROOKEY_UI_LANG=pl ./target/release/rookey ui
```

People pick the language on the settings page. Without a choice, rookey follows the system (`LANG`) on Linux, and on the settings page it follows the browser.
