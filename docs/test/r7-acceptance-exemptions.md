---
title: "R7 exemptions — acceptance citing a function production never calls"
status: Draft
version: "0.1"
updated: 2026-09-29
authors:
  - Simon Keimer (DC0SK)
---

# R7 exemptions — acceptance citing a function production never calls

Rule **R7** (`cargo xtask`, NFR-TEST-01): a requirement's **acceptance** cell must not name, as
code in backticks, a function that rule R6 finds unreferenced by production. Such a test passes
whatever the product does: `FR-PAN-06/07/08` were evidenced that way until #229. Only the
acceptance cell is read, so a test that uses a test seam while it exercises production is not
flagged.

An exemption is one line, `` - `ID:function` — reason ``, for a function that is the right
evidence although the product never calls it, such as a measuring instrument. An exemption that
no longer applies (the row stopped citing the function, or production calls it now) fails the
build, like a stale R6 baseline entry.

No exemptions today.
