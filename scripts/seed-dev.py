#!/usr/bin/env python3
"""Sinh mock data cho dev server: habits + logs, board kanban demo, pass admin.

Chạy:  python3 scripts/seed-dev.py | docker exec -i nk-pg-test psql -U nk -d nk -v ON_ERROR_STOP=1
Idempotent: xoá habits của admin + board 'Demo' trước khi chèn lại.
"""
import datetime
import random
import time
import uuid

from argon2 import PasswordHasher

DEV_PASSWORD = "dev12345"
TODAY = datetime.date.today()  # máy dev để giờ VN
NOW = int(time.time())


def noon_ts(d: datetime.date) -> int:
    return int(datetime.datetime(d.year, d.month, d.day, 12, 0, 0, tzinfo=datetime.timezone.utc).timestamp())


def daterange(a: datetime.date, b: datetime.date):
    d = a
    while d <= b:
        yield d
        d += datetime.timedelta(days=1)


random.seed(7)

# (title, created, cach chon ngay log)
HABITS = [
    ("Learn Chinese", datetime.date(2026, 8, 1), {"skip_nov": {3, 7, 15}, "old_p": 0.65}),
    ("Hành Thư", datetime.date(2026, 8, 1), {"nov_fixed": {2, 5, 9, 14, 24}, "old_p": 0.10}),
    ("Khải Thư", datetime.date(2026, 8, 1), {"nov_fixed": {1, 24}, "old_p": 0.08}),
    ("Learn Robot", datetime.date(2026, 10, 15), {"nov_p": 0.5, "old_p": 0.4}),
    ("Learn Web", datetime.date(2026, 10, 15), {"nov_p": 0.5, "old_p": 0.4}),
    ("Learn ESP32", datetime.date(2026, 10, 15), {"nov_p": 0.5, "old_p": 0.4}),
    ("Learn CUDA", datetime.date(2026, 10, 15), {"nov_p": 0.5, "old_p": 0.4}),
    ("Learn LLM + Agent", TODAY, {"nov_fixed": {TODAY.day}}),
    ("Daily Learning", TODAY - datetime.timedelta(days=4), {"nov_all": True}),
]

out = []
out.append("-- seed dev: %s" % TODAY.isoformat())
ph = PasswordHasher()
out.append("UPDATE users SET pass_hash = '%s', must_reset = false WHERE username = 'admin';" % ph.hash(DEV_PASSWORD))
out.append("DELETE FROM habits WHERE owner = (SELECT id FROM users WHERE username = 'admin');")
out.append("DELETE FROM boards WHERE owner = (SELECT id FROM users WHERE username = 'admin') AND title = 'Demo';")

nov_start = datetime.date(TODAY.year, TODAY.month, 1)
for title, created, rule in HABITS:
    hid = uuid.uuid4()
    out.append(
        "INSERT INTO habits (id, owner, title, schedule, created_at) VALUES "
        "('%s', (SELECT id FROM users WHERE username = 'admin'), $$%s$$, 'daily', %d);"
        % (hid, title, NOW if created == TODAY else noon_ts(created))
    )
    for d in daterange(max(created, TODAY - datetime.timedelta(days=400)), TODAY):
        ds = d.isoformat()
        keep = False
        if d >= nov_start:
            if rule.get("nov_all"):
                keep = True
            elif "nov_fixed" in rule:
                keep = d.day in rule["nov_fixed"]
            elif "skip_nov" in rule:
                keep = d.day not in rule["skip_nov"]
            else:
                keep = random.random() < rule.get("nov_p", 0.5)
        else:
            keep = random.random() < rule.get("old_p", 0)
        if keep:
            out.append("INSERT INTO habit_logs (habit_id, day, created_at) VALUES ('%s', '%s', %d) ON CONFLICT DO NOTHING;" % (hid, ds, noon_ts(d)))

# Board demo cho task tracker: quá hạn / hôm nay / tuần này / đã xong
bid, c_todo, c_doing, c_done = (uuid.uuid4() for _ in range(4))
out.append(
    "INSERT INTO boards (id, owner, title, created_at) VALUES "
    "('%s', (SELECT id FROM users WHERE username = 'admin'), 'Demo', %d);" % (bid, NOW)
)
out.append("INSERT INTO board_columns (id, board_id, title, position, is_done) VALUES ('%s', '%s', 'Todo', 0, false);" % (c_todo, bid))
out.append("INSERT INTO board_columns (id, board_id, title, position, is_done) VALUES ('%s', '%s', 'Doing', 1, false);" % (c_doing, bid))
out.append("INSERT INTO board_columns (id, board_id, title, position, is_done) VALUES ('%s', '%s', 'Done', 2, true);" % (c_done, bid))


def card(col, title, pos, due=None, done=None, parent=None):
    cid = uuid.uuid4()
    out.append(
        "INSERT INTO cards (id, board_id, column_id, parent_id, title, position, due_at, done_at, created_at, updated_at) "
        "VALUES ('%s', '%s', '%s', %s, $$%s$$, %d, %s, %s, %d, %d);"
        % (cid, bid, col, ("'%s'" % parent) if parent else "NULL", title, pos,
           due if due is not None else "NULL", done if done is not None else "NULL", NOW, NOW)
    )
    return cid


parent = card(c_todo, "Habit matrix polish", 0, NOW + 3 * 86400)
card(c_todo, "Fix login bug", 1, NOW - 90000)
card(c_todo, "Release 0.1.53", 2, NOW + 3 * 86400)
card(c_todo, "Subtask: week colors", 3, NOW + 86400, None, parent)
card(c_doing, "Review habit UI", 0, NOW + 7200)
card(c_done, "Deploy 0.1.52", 0, NOW - 2 * 86400, NOW - 86400)

print("\n".join(out))
print("-- admin/dev password: %s" % DEV_PASSWORD, flush=True)
