# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Generate local proposal data and UI accessors without modifying published sources."""
import hashlib
import json
from pathlib import Path
from drafts import DRAFTS

ROOT = Path(__file__).resolve().parent
LOCAL = ROOT / '.local'


def build():
    inventory = json.loads((LOCAL / 'inventory.json').read_text())
    for segment in inventory:
        options = []
        for index, (label, values) in enumerate(DRAFTS[segment['id']]):
            if len(values) != len(segment['fields']):
                raise ValueError(f"Field count changed: {segment['id']} / {label}")
            options.append({
                'id': chr(65 + index), 'label': label,
                'values': dict(zip([f['id'] for f in segment['fields']], values)),
            })
        segment['options'] = options
    payload = {**json.loads((LOCAL / 'source.json').read_text()), 'segments': inventory}
    payload['revision'] = hashlib.sha256(json.dumps(payload, sort_keys=True).encode()).hexdigest()

    # A new baseline or revised alternatives start a new review. Preserve the previous
    # choices and their proposal text before replacing generated files. Do not silently
    # carry approvals forward onto different copy, even if the section IDs still match.
    state_file = LOCAL / 'review-state.json'
    if state_file.exists():
        state = json.loads(state_file.read_text())
        if state['revision'] != payload['revision']:
            archive = LOCAL / 'history' / f"{state['revision']}-{state['version']}"
            archive.mkdir(parents=True, exist_ok=True)
            for name in ('proposals.json', 'review-state.json'):
                source = LOCAL / name
                if source.exists():
                    (archive / name).write_bytes(source.read_bytes())
            state_file.unlink()
    (LOCAL / 'proposals.json').write_text(json.dumps(payload, ensure_ascii=False, indent=2) + '\n')

    # Review controls are app-owned text; use generated named accessors in review.js.
    # Proposed marketing text is publication content and stays in the proposal data.
    strings = json.loads((ROOT / 'strings.en.json').read_text())
    js = '// Generated from strings.en.json by build.py.\nexport const res = { str: {\n'
    for key, value in strings.items():
        js += (f'  {key}: (args = {{}}) => {json.dumps(value, ensure_ascii=False)}'
               '.replace(/\\{(\\w+)\\}/g, (_, key) => String(args[key] ?? "")),\n')
    js += '} };\n'
    (LOCAL / 'res.mjs').write_text(js)
    print(f"Prepared {len(inventory)} sections; review {payload['revision'][:12]}.")


if __name__ == '__main__':
    build()
