# Writes scripts/config-list.txt and expect/config-list.expect from the /config table Settings >
# Claude Code > Advanced writes (crates/core/src/cc_config.rs). Run after changing that table:
#
#   python -I tools/screen-recorder/config-list.py
#
# An entry in the table's WHEN list (shown by /config only in some contexts) may be missing from a
# recording: its expectation accepts /config's "No settings match" instead. That its label still
# exists is checked by tools/cli-compat.ps1 (the keys check), which needs no recording.
import pathlib
import re

repo = pathlib.Path(__file__).resolve().parents[2]
src = (repo / 'crates/core/src/cc_config.rs').read_text(encoding='utf-8')
body = src[src.index('pub const ENTRIES'):src.index('pub const WHEN')]
items = re.findall(r'(?:switch|entry)\(\s*"([^"]+)",\s*"([^"]+)"', body)
when_block = src[src.index('pub const WHEN'):src.index('impl Entry')]
when = dict(re.findall(r'\("([^"]+)",\s*"([^"]+)"\)', when_block))
assert set(when) <= {k for k, _ in items}, 'WHEN names a key the table lacks'

BS = '\\'
SEARCH = '\u2315 Search settings'
lines = [
    '# Searches /config for every entry Settings › Claude Code › Advanced writes and snaps each',
    '# result: a label that no longer shows means a renamed or removed entry. Written by',
    '# ../config-list.py from crates/core/src/cc_config.rs. The search text is the label in lower',
    '# case, so the label as written shows only in the list. Each search first needs the empty',
    '# search box on screen, so no key goes anywhere else. No model request; nothing is changed.',
    '#: size 160 50',
    '#: seconds %d' % (10 + len(items) * 2),
    'start',
    'send /config',
    'sleep 800',
    'send ' + BS + 'r',
    # Typed text goes into /config's search box only while it shows, empty (its placeholder after
    # the search mark); anywhere else a space could toggle a setting. Without it the script ends.
    'need ' + SEARCH + ' 5000',
]
exp = [
    '# The labels scripts/config-list.txt searches /config for. Written by ../config-list.py from',
    '# crates/core/src/cc_config.rs.',
]
for key, label in items:
    name = 'cfg-' + re.sub(r'[^a-z0-9]+', '-', key.lower())
    term = label.lower()
    lines += [
        'need ' + SEARCH + ' 3000',
        'send ' + term.replace(' ', BS + 's'),
        'sleep 900',
        'snap ' + name,
        'send ' + (BS + 'x7f') * len(term),
        'sleep 500',
    ]
    exp += ['', '[%s]' % name]
    if key in when:
        exp += ['# Listed only in some contexts (%s): missing is fine.' % when[key].rstrip('.'),
                'shows-any = %s | No settings match' % label]
    else:
        exp += ['shows = ' + label]
lines += ['send ' + BS + 'e', 'sleep 800', 'send ' + BS + 'e', 'sleep 1500', 'snap config-closed']
exp += ['', '[config-closed]', 'kind = idle']
for path, text in [('scripts/config-list.txt', lines), ('expect/config-list.expect', exp)]:
    with open(repo / 'tools/screen-recorder' / path, 'w', encoding='utf-8', newline='\n') as f:
        f.write('\n'.join(text) + '\n')
print(len(items), 'entries,', len(when), 'only in some contexts')
