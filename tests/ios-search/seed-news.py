#!/usr/bin/env python3
"""Seed only the disposable search-drawer fixture, after its first Day-News launch."""
from pathlib import Path
import sqlite3
import time

path = Path('/tmp/day-search-drawer-fixture/sheets2.sqlite3')
if not path.exists():
    raise SystemExit('Launch Day-News once with DAY_DATA_DIR=/tmp/day-search-drawer-fixture first')
now = int(time.time())
with sqlite3.connect(path) as db:
    for index in range(40):
        feed = 980000 + index
        db.execute('''INSERT OR REPLACE INTO feeds
            (id, feed_url, title, unsubscribed, poll_failures, position, next_poll_at)
            VALUES (?, ?, ?, 0, 0, ?, ?)''',
            (feed, f'asset:demo/search-fixture-{index}.xml', f'Search fixture {index + 1:02}', index, 4102444800))
        db.execute('''INSERT OR REPLACE INTO articles
            (id, guid, feed, title, published_at, is_read, is_starred, first_seen_at)
            VALUES (?, ?, ?, ?, ?, 0, 0, ?)''',
            (feed, f'search-fixture-{index}', feed, f'Synthetic search article {index + 1:02}', now, now))
print('Prepared 40 offline feeds in the isolated search fixture')
