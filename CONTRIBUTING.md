# Contributing

## Licence of contributions

The engine is dual-licensed **MIT OR Apache-2.0**. Contributions are accepted under the same terms:
unless you state otherwise, anything you submit is offered under both, and you keep the copyright in
your own work.

There is no copyright assignment and no CLA. The consequence is worth stating plainly: a future change
of licence would need every contributor's agreement. Dual MIT/Apache-2.0 is permissive enough that this
should never be necessary, which is exactly why it was chosen over a CLA.

## Developer Certificate of Origin

Every commit must carry a `Signed-off-by` line certifying you have the right to submit it. `git commit
-s` adds one:

```
Signed-off-by: Your Name <your.email@example.com>
```

The name must be a real name and the address a reachable one. By signing off you certify the
Developer Certificate of Origin 1.1:

> By making a contribution to this project, I certify that:
>
> (a) The contribution was created in whole or in part by me and I have the right to submit it under
> the open source license indicated in the file; or
>
> (b) The contribution is based upon previous work that, to the best of my knowledge, is covered under
> an appropriate open source license and I have the right under that license to submit that work with
> modifications, whether created in whole or in part by me, under the same open source license (unless
> I am permitted to submit under a different license), as indicated in the file; or
>
> (c) The contribution was provided directly to me by some other person who certified (a), (b) or (c)
> and I have not modified it.
>
> (d) I understand and agree that this project and the contribution are public and that a record of the
> contribution (including all personal information I submit with it, including my sign-off) is
> maintained indefinitely and may be redistributed consistent with this project or the open source
> license(s) involved.

The full text is at <https://developercertificate.org/>.

## Provenance: the rule that matters most here

**Do not port, translate, or transcribe code, data, or constants from another game.** Not from a
decompilation, not from a reverse-engineered project, not from a wiki that documents one.

This is not a stylistic preference. The engine's predecessor was locked to GPL-3.0-only because parts of
it derived from a GPL reimplementation of a commercial game; the current tree is permissively licensed
*only* because that derivation was removed file by file. A single ported constant table would put the
whole distributed work back under obligations MIT and Apache-2.0 cannot satisfy.

Two specific files were held back from the seed for this reason and still need writing from scratch —
water rendering and scenery sway. If you implement either, do not consult the originals. See
[LICENSING.md](LICENSING.md).

Reimplementing a *published* technique is fine and encouraged; cite the paper. The ambient-occlusion
pass does exactly that.

## The gate

Every change must pass, on the pinned toolchain:

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

`unsafe_code` is forbidden at workspace scope. Lints are errors, not warnings — a warning nobody fixes
becomes noise that hides the next real one.

## Two standing rules earned the hard way

**A green test suite is not verification for a rendering change.** Look at the capture. Every rendering
bug in this project so far passed its own assertions: reversed layer weights that made a layer
invisible, two separate tone-mapping mistakes that flattened all contrast, a shadow camera placed on the
dark side of the scene, an occlusion blur whose tolerance rejected every neighbour at distance, and
twice a test fixture that measured itself rather than the renderer. The render tests write PNGs to
`target/tmp/` for this reason.

**Presentation needs running, not only testing.** The one bug the headless suite structurally could not
catch — surface capabilities queried through an adapter belonging to a different instance — appeared the
first time a window opened.

## Tests

Unit tests sit beside the code they cover. Fixtures are *built*, not committed as binary blobs: the zip,
tar, and glTF tests construct real containers at test time, so a test states the structural case it
cares about rather than pointing at an opaque file.

A test that cannot fail for the reason it claims is worse than no test. Prefer a control that differs in
exactly one variable — see how the shadow test isolates shadowing by collapsing the shadow distance
rather than by moving the sun.
