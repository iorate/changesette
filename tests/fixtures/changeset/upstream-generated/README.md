# Fixture provenance

`olive-pots-repeat.md` is byte-exact output of the original `@changesets/cli`.
The format was taken from `@changesets/write@0.4.0` (`writeChangeset`):

```js
const changesetContents = `---
${releases.map(release => `"${release.name}": ${release.type}`).join("\n")}
---

${summary}
  `;
```

which is then run through `prettier` with `parser: "markdown"`, normalising the
trailing whitespace to a single newline. The quoting of package names is
deliberate upstream ("the quotation marks in here are really important even
though they are not spec for yaml"), and is what `changeset::strip_quotes`
exists to absorb.

This file also doubles as the fixture for the `README.md` exclusion rule (§4).
