---
changesette: major
---

Packages whose package.json has no version field are now included as workspace members, and get-packages reports a private field for every package and omits version when the package.json has none.
