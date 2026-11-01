# License and attribution

Reviewed 2026-10-31 against the [Stepsy license](https://github.com/nvllz/stepsy/blob/2ff512d34dcd858688e64a83f6c2fc1d778f75f6/LICENSE), its [README](https://github.com/nvllz/stepsy/blob/2ff512d34dcd858688e64a83f6c2fc1d778f75f6/README.md), and the [F-Droid license metadata](https://f-droid.org/packages/com.nvllz.stepsy/), which identifies **GPL-3.0-only**.

## Recommended repository relationship

Use a new, independently maintained repository with its own name and application identity. Android services, permissions, views, build tooling and widgets are not reusable as a native GNOME application. Keeping an Android fork network and its complete build tree offers little practical benefit for this port.

A separate repository does not change copyright obligations. Translated or adapted implementation remains subject to the original license. See the [GNU licensing FAQ on translating code](https://www.gnu.org/licenses/gpl-faq.html#TranslateCode). A GitHub fork is a hosting relationship, not a distinct license category.

The license of this project is GPL-3.0-only, including original project documentation and artwork unless a file states otherwise. Do not silently relabel adapted code as a permissive license or assume permission to use later GPL versions. Preserve the original terms of third-party components.

## Origins

- Stepsy: [nvllz/stepsy](https://github.com/nvllz/stepsy), maintained by nvllz.
- Earlier project acknowledged by Stepsy: [MotionMate](https://github.com/0xf4b1/motionmate), by 0xf4b1.
- Baseline source commit: `2ff512d34dcd858688e64a83f6c2fc1d778f75f6`.

No application code, translations, screenshots, fonts, badges or icons from these projects are included in this repository. The GPL license document is reproduced verbatim. The icon is original geometry.

## Third-party components

The binaries statically link Rust crates and SQLite. `packaging/third-party-licenses.sh` collects the name, version, license and license text of every linked crate. The result is installed as `/usr/share/doc/pace-beads/THIRD-PARTY-LICENSES` and attached to each release.

## Requirements when incorporating source or assets

1. Record each copied or adapted file, upstream path, exact commit, original copyright holders and applicable license. Preserve notices and disclaimers in the file or an appropriate accompanying attribution record; do not replace earlier authors with the current owner.
2. Mark modified files prominently with the nature and date of the changes as required by GPL section 5. Record translated implementations as adaptations, not as wholly original source.
3. Include the complete GPL text and appropriate legal notices in distributions and the application's About information. Observe applicable interactive legal-notice obligations.
4. When distributing binaries, provide the Corresponding Source under an appropriate GPL section 6 mechanism. Prefer exact tagged source archives alongside release packages, including build and installation scripts. A vague link to a moving branch is insufficient as a release record. Include Installation Information where the license requires it for a User Product.
5. Review assets and dependencies individually. A repository's top-level license does not resolve every bundled font, library or artwork license. Stepsy lists MPAndroidChart under Apache-2.0; no Android chart library is needed for this native UI. Do not bring in the proprietary Full-edition dependencies.
6. Check SensorFW/client code licenses before copying any helpers. The examined SensorFW adaptor carries LGPL-2.1 terms and the Andromeda client sample carries GPL version 3 terms; their notices must be retained if reused.
7. Use a separate name and icon and describe the project as unofficial. A source-code license does not grant trademark rights or endorsement. No assumption of permission to reuse Stepsy branding is made.

GPL allows compliant modification and redistribution without separate author permission. An optional courtesy message to upstream is independent of compliance and would require the owner's instruction to send it.

## Before publication

Review the exact staged files, their licenses and provenance. Do not remove required third-party notices to satisfy editorial preferences. At this stage there are no incorporated application files to list in an adaptation ledger; create that ledger with the first actual reuse.
