#!/usr/bin/env python3
"""Xuất Nextcloud Deck từ backup SQLite ra JSON để NextKloud nhập (Kanban → Nhập từ Deck).

Dùng: python3 deck_export.py <nextcloud.db> <owner> <out.json>
Chỉ đọc DB (mode=ro). Bỏ card/cột/board đã xoá.
"""
import calendar
import json
import sqlite3
import sys
import time


def ts(v):
    """'2025-04-21 01:00:00' (UTC) -> epoch giây."""
    if not v:
        return None
    return calendar.timegm(time.strptime(v[:19], "%Y-%m-%d %H:%M:%S"))


def main(db, owner, out):
    c = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    c.row_factory = sqlite3.Row
    boards = []
    # File đính kèm (Deck >= 1.3): share kiểu 12 (deck), share_with = id card, file trong thư mục của owner.
    att = {}
    for card_id, path in c.execute(
        "SELECT s.share_with, f.path FROM oc_share s JOIN oc_filecache f ON f.fileid = s.file_source "
        "JOIN oc_storages st ON st.numeric_id = f.storage WHERE s.share_type = 12 AND st.id = ? AND f.path LIKE 'files/%'",
        (f"home::{owner}",)):
        att.setdefault(int(card_id), []).append(path[len("files/"):])
    for b in c.execute("SELECT id, title, archived FROM oc_deck_boards WHERE owner = ? AND deleted_at = 0 ORDER BY id", (owner,)):
        labels = {r["id"]: (r["title"], "#" + (r["color"] or "888888")) for r in c.execute("SELECT id, title, color FROM oc_deck_labels WHERE board_id = ?", (b["id"],))}
        stacks = [dict(id=s["id"], title=s["title"], order=s["order"]) for s in c.execute(
            'SELECT id, title, "order" FROM oc_deck_stacks WHERE board_id = ? AND deleted_at = 0 ORDER BY "order", id', (b["id"],))]
        cards = []
        for k in c.execute(
            'SELECT k.* FROM oc_deck_cards k JOIN oc_deck_stacks s ON s.id = k.stack_id '
            'WHERE s.board_id = ? AND s.deleted_at = 0 AND k.deleted_at = 0 ORDER BY k."order", k.id', (b["id"],)):
            lab = [labels[r[0]][0] for r in c.execute("SELECT label_id FROM oc_deck_assigned_labels WHERE card_id = ?", (k["id"],)) if r[0] in labels]
            cards.append(dict(
                id=k["id"], title=k["title"], description=k["description"] or "", stack_id=k["stack_id"], order=k["order"],
                created_at=k["created_at"], last_modified=k["last_modified"], due_at=ts(k["duedate"]), done_at=ts(k["done"]),
                archived=bool(k["archived"]), labels=lab, color=("#" + k["color"]) if k["color"] else None,
                attachments=sorted(att.get(k["id"], [])),
            ))
        boards.append(dict(id=b["id"], title=b["title"], archived=bool(b["archived"]),
                           labels=[dict(title=t, color=col) for t, col in labels.values()], stacks=stacks, cards=cards))
    with open(out, "w") as f:
        json.dump(dict(source="nextcloud-deck", boards=boards), f, ensure_ascii=False)
    n_att = sum(len(k["attachments"]) for b in boards for k in b["cards"])
    print(f"{len(boards)} board, {sum(len(b['cards']) for b in boards)} card, {n_att} đính kèm -> {out}")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
