<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Documentation page templates

Copy the relevant template and replace every instruction with verified content. These files are
unpublished authoring aids, not runnable examples or approved public route proposals. Use the
[style guide](../style-guide.md) and [authoring workflow](../authoring.md).

| Template | Primary purpose |
| --- | --- |
| [Tutorial](tutorial.md) | Learn through a complete working sequence |
| [How-to](how-to.md) | Complete one concrete task |
| [Reference](reference.md) | Look up an exact contract |
| [Explanation](explanation.md) | Understand a model or tradeoff |
| [Product](product.md) | Evaluate a specific capability with evidence |

For website app docs, replace the template H1 with front matter containing `title` and
`description`, then set the existing `section` and `order` fields deliberately. Do not add
unimplemented schema fields. For canonical internal docs, keep one H1 and add the symlink and
curation entry described in the website instructions. Put audience, type, implementation status,
and verification evidence in a short author note or linked review record; these are authoring
requirements, not new Astro metadata fields.

Label a proposed design before any API description. Do not fill placeholder code with guessed
methods. Include the full resource setup for runnable examples and label partial snippets.
