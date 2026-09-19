---
title: "Web view JS evaluation"
description: "The web_eval dayscript compatibility command and the external webview documentation."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Web view JavaScript evaluation

The webview API, JavaScript reply format, and platform implementation notes now live in
[day-piece-webview's evaluation guide](https://github.com/daybrite/day-piece-webview/blob/main/docs/webview-eval.md).

Dayscript keeps the `web_eval` command for existing scripts. It calls the named piece operation
`day.webview.eval`, passing the script as text and expecting the result as JSON text. The piece
registers that operation for its node kind; Day's core registry has no browser-specific types or
state. See [piece operations](extending.md#named-piece-operations) for the shared interface.

```yaml
- web_eval:
    id: reader-web
    script: "document.title"
    text: "My page"
    timeout_secs: 30
```

`text` compares the returned string or JSON representation; `contains` checks a substring.
The runner polls pending replies and retries mismatches within the step's wait period (five
seconds by default). Set `timeout_secs` for cold engine startup or slow page loads; it extends
both the engine's polling budget and the runner's reply timeout without delaying a ready page. An absent
operation fails immediately. Gate the step with `only_on` for platforms that support evaluation.
