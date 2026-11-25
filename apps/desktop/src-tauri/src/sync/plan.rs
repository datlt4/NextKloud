//! Quyết định 3 chiều cho đồng bộ thư mục: hàm thuần, không IO (test được).
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Thư mục rác local (file bị xoá từ server được dời vào đây), nằm trong gốc, không đồng bộ.
pub const TRASH: &str = ".nextkloud-trash";
/// Đuôi file đang tải dở của chính app.
pub const PART: &str = ".nkpart";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
}

impl Meta {
    pub fn file(size: u64, mtime: i64) -> Self {
        Meta { is_dir: false, size, mtime }
    }
    pub fn dir() -> Self {
        Meta { is_dir: true, size: 0, mtime: 0 }
    }
}

/// Trạng thái lần đồng bộ trước của 1 path: (size, mtime) ở máy và ở server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    #[serde(rename = "d", default)]
    pub is_dir: bool,
    #[serde(default)]
    pub ls: u64,
    #[serde(default)]
    pub lm: i64,
    #[serde(default)]
    pub rs: u64,
    #[serde(default)]
    pub rm: i64,
}

impl Base {
    pub fn dir() -> Self {
        Base { is_dir: true, ls: 0, lm: 0, rs: 0, rm: 0 }
    }
    pub fn of(l: Meta, r: Meta) -> Self {
        Base { is_dir: l.is_dir, ls: l.size, lm: l.mtime, rs: r.size, rm: r.mtime }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Upload,
    Download,
    /// Xoá trên server (vào thùng rác server); thư mục thì xoá cả cây.
    DeleteRemote,
    /// Dời bản local vào `.nextkloud-trash/`; thư mục thì dời cả cây.
    TrashLocal,
    /// Cả 2 bên cùng sửa: bản server giữ tên gốc, bản máy đổi tên conflict rồi upload.
    Conflict,
    MkdirRemote,
    MkdirLocal,
    /// 2 bên đã giống nhau: chỉ ghi lại state.
    Record,
    /// 2 bên đều không còn: bỏ khỏi state.
    Forget,
    /// File ở bên này, thư mục ở bên kia: bỏ qua, báo lỗi.
    TypeClash,
}

/// Bỏ qua: file tạm Office/LibreOffice, rác hệ điều hành, file đang tải dở, thùng rác local.
pub fn ignored(rel: &str) -> bool {
    if rel == TRASH || rel.starts_with(".nextkloud-trash/") {
        return true;
    }
    rel.split('/').any(|n| {
        let l = n.to_ascii_lowercase();
        n.starts_with("~$")
            || n.starts_with(".~lock.")
            || l.ends_with(".tmp")
            || l.ends_with(PART)
            || n == ".DS_Store"
            || l == "thumbs.db"
            || n.is_empty()
            || n == "."
            || n == ".."
    })
}

fn same(m: Meta, size: u64, mtime: i64) -> bool {
    m.size == size && m.mtime == mtime
}

/// Quyết định cho 1 path. None = không phải làm gì.
pub fn decide(l: Option<Meta>, r: Option<Meta>, b: Option<Base>) -> Option<Action> {
    use Action::*;
    // base khác loại (file thành thư mục...) coi như chưa từng đồng bộ
    let kind = l.or(r).map(|m| m.is_dir);
    let b = b.filter(|b| kind.is_none_or(|k| k == b.is_dir));
    match (l, r) {
        (None, None) => b.map(|_| Forget),
        (Some(l), Some(r)) if l.is_dir != r.is_dir => Some(TypeClash),
        (Some(l), Some(_)) if l.is_dir => (b.is_none()).then_some(Record),
        (Some(l), Some(r)) => match b {
            None if same(l, r.size, r.mtime) => Some(Record),
            None => Some(Conflict),
            Some(b) => {
                let lc = !same(l, b.ls, b.lm);
                let rc = !same(r, b.rs, b.rm);
                match (lc, rc) {
                    (false, false) => None,
                    (true, false) => Some(Upload),
                    (false, true) => Some(Download),
                    (true, true) if same(l, r.size, r.mtime) => Some(Record),
                    (true, true) => Some(Conflict),
                }
            }
        },
        (Some(l), None) => match b {
            None if l.is_dir => Some(MkdirRemote),
            None => Some(Upload),
            Some(_) if l.is_dir => Some(TrashLocal),
            // server đã xoá: bản máy có sửa thì giữ bản sửa
            Some(b) if !same(l, b.ls, b.lm) => Some(Upload),
            Some(_) => Some(TrashLocal),
        },
        (None, Some(r)) => match b {
            None if r.is_dir => Some(MkdirLocal),
            None => Some(Download),
            Some(_) if r.is_dir => Some(DeleteRemote),
            Some(b) if !same(r, b.rs, b.rm) => Some(Download),
            Some(_) => Some(DeleteRemote),
        },
    }
}

/// Kế hoạch cho cả cây. Xoá thư mục được gộp thành 1 thao tác nếu mọi thứ bên trong đều bị xoá;
/// còn thứ cần giữ (bên kia vừa sửa/thêm) thì thư mục được tạo lại thay vì xoá.
pub fn plan(local: &BTreeMap<String, Meta>, remote: &BTreeMap<String, Meta>, base: &BTreeMap<String, Base>) -> BTreeMap<String, Action> {
    let keys: BTreeSet<&String> = local.keys().chain(remote.keys()).chain(base.keys()).collect();
    let mut out: BTreeMap<String, Action> = BTreeMap::new();
    for k in keys {
        if let Some(a) = decide(local.get(k).copied(), remote.get(k).copied(), base.get(k).copied()) {
            out.insert(k.clone(), a);
        }
    }
    // thư mục sâu trước để thư mục cha thấy kết quả đã gộp của con
    let dirs: Vec<(String, Action)> = out
        .iter()
        .rev()
        .filter(|(k, a)| matches!(a, Action::DeleteRemote | Action::TrashLocal) && (local.get(*k).or(remote.get(*k))).is_some_and(|m| m.is_dir))
        .map(|(k, a)| (k.clone(), *a))
        .collect();
    for (d, a) in dirs {
        let pre = format!("{d}/");
        let kids: Vec<String> = out.range(pre.clone()..).take_while(|(k, _)| k.starts_with(&pre)).map(|(k, _)| k.clone()).collect();
        let all_gone = kids.iter().all(|k| matches!(out[k], Action::Forget) || out[k] == a);
        if all_gone {
            for k in kids {
                out.remove(&k);
            }
        } else {
            out.insert(d, if a == Action::DeleteRemote { Action::MkdirLocal } else { Action::MkdirRemote });
        }
    }
    out
}

/// "a/b/report.docx" -> "a/b/report (conflict host 2026-11-26 1530).docx".
pub fn conflict_name(rel: &str, host: &str, stamp: &str) -> String {
    let (dir, name) = rel.rsplit_once('/').map(|(d, n)| (format!("{d}/"), n)).unwrap_or((String::new(), rel));
    let tag = format!(" (conflict {host} {stamp})");
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{dir}{stem}{tag}.{ext}"),
        _ => format!("{dir}{name}{tag}"),
    }
}

/// Giây unix (đã cộng lệch múi giờ) -> "yyyy-mm-dd hhmm".
pub fn stamp(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // thuật toán civil_from_days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}{:02}", rem / 3600, rem % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::*;

    const F: fn(u64, i64) -> Meta = Meta::file;
    fn b(ls: u64, lm: i64, rs: u64, rm: i64) -> Option<Base> {
        Some(Base { is_dir: false, ls, lm, rs, rm })
    }

    #[test]
    fn decide_table() {
        type Case = (&'static str, Option<Meta>, Option<Meta>, Option<Base>, Option<Action>);
        let cases: Vec<Case> = vec![
            ("mới ở máy", Some(F(1, 10)), None, None, Some(Upload)),
            ("mới ở server", None, Some(F(1, 10)), None, Some(Download)),
            ("2 bên mới giống hệt", Some(F(5, 10)), Some(F(5, 10)), None, Some(Record)),
            ("2 bên mới khác nhau", Some(F(5, 10)), Some(F(6, 11)), None, Some(Conflict)),
            ("không đổi", Some(F(5, 10)), Some(F(5, 10)), b(5, 10, 5, 10), None),
            ("sửa ở máy", Some(F(6, 20)), Some(F(5, 10)), b(5, 10, 5, 10), Some(Upload)),
            ("sửa ở server", Some(F(5, 10)), Some(F(7, 30)), b(5, 10, 5, 10), Some(Download)),
            ("chỉ mtime đổi ở máy", Some(F(5, 11)), Some(F(5, 10)), b(5, 10, 5, 10), Some(Upload)),
            ("2 bên cùng sửa", Some(F(6, 20)), Some(F(7, 30)), b(5, 10, 5, 10), Some(Conflict)),
            ("2 bên cùng sửa ra giống nhau", Some(F(6, 20)), Some(F(6, 20)), b(5, 10, 5, 10), Some(Record)),
            ("xoá ở máy", None, Some(F(5, 10)), b(5, 10, 5, 10), Some(DeleteRemote)),
            ("xoá ở server", Some(F(5, 10)), None, b(5, 10, 5, 10), Some(TrashLocal)),
            ("xoá ở máy, server sửa -> giữ bản sửa", None, Some(F(9, 40)), b(5, 10, 5, 10), Some(Download)),
            ("xoá ở server, máy sửa -> giữ bản sửa", Some(F(9, 40)), None, b(5, 10, 5, 10), Some(Upload)),
            ("2 bên đều xoá", None, None, b(5, 10, 5, 10), Some(Forget)),
            ("file vs thư mục", Some(F(1, 1)), Some(Meta::dir()), None, Some(TypeClash)),
            ("thư mục mới ở máy", Some(Meta::dir()), None, None, Some(MkdirRemote)),
            ("thư mục mới ở server", None, Some(Meta::dir()), None, Some(MkdirLocal)),
            ("thư mục 2 bên, chưa có state", Some(Meta::dir()), Some(Meta::dir()), None, Some(Record)),
            ("thư mục 2 bên, đã có state", Some(Meta::dir()), Some(Meta::dir()), Some(Base::dir()), None),
            ("thư mục xoá ở máy", None, Some(Meta::dir()), Some(Base::dir()), Some(DeleteRemote)),
            ("thư mục xoá ở server", Some(Meta::dir()), None, Some(Base::dir()), Some(TrashLocal)),
            // state cũ là file, giờ máy có thư mục cùng tên: coi như mới
            ("file thành thư mục", Some(Meta::dir()), None, b(1, 1, 1, 1), Some(MkdirRemote)),
        ];
        for (name, l, r, base, want) in cases {
            assert_eq!(decide(l, r, base), want, "{name}");
        }
    }

    fn map<T: Copy>(v: &[(&str, T)]) -> BTreeMap<String, T> {
        v.iter().map(|(k, m)| (k.to_string(), *m)).collect()
    }

    #[test]
    fn plan_collapses_dir_delete() {
        let base = map(&[("d", Base::dir()), ("d/a", b(1, 1, 1, 1).unwrap()), ("d/s", Base::dir()), ("d/s/b", b(2, 2, 2, 2).unwrap())]);
        // xoá cả thư mục ở máy -> 1 lệnh xoá trên server
        let remote = map(&[("d", Meta::dir()), ("d/a", F(1, 1)), ("d/s", Meta::dir()), ("d/s/b", F(2, 2))]);
        let p = plan(&BTreeMap::new(), &remote, &base);
        assert_eq!(p, map(&[("d", DeleteRemote)]));
        // server sửa 1 file bên trong -> không xoá thư mục, tải bản sửa về
        let remote2 = map(&[("d", Meta::dir()), ("d/a", F(1, 1)), ("d/s", Meta::dir()), ("d/s/b", F(3, 9))]);
        let p = plan(&BTreeMap::new(), &remote2, &base);
        assert_eq!(p, map(&[("d", MkdirLocal), ("d/a", DeleteRemote), ("d/s", MkdirLocal), ("d/s/b", Download)]));
        // server xoá cả thư mục -> dời cả thư mục local vào rác
        let local = map(&[("d", Meta::dir()), ("d/a", F(1, 1)), ("d/s", Meta::dir()), ("d/s/b", F(2, 2))]);
        let p = plan(&local, &BTreeMap::new(), &base);
        assert_eq!(p, map(&[("d", TrashLocal)]));
        // ... nhưng máy vừa thêm file mới trong đó -> giữ, upload lại
        let mut local2 = local.clone();
        local2.insert("d/s/new".into(), F(4, 4));
        let p = plan(&local2, &BTreeMap::new(), &base);
        assert_eq!(p, map(&[("d", MkdirRemote), ("d/a", TrashLocal), ("d/s", MkdirRemote), ("d/s/b", TrashLocal), ("d/s/new", Upload)]));
    }

    #[test]
    fn plan_sibling_prefix_not_child() {
        // "d" bị xoá ở máy; "d x" và "d.txt" không phải con của "d"
        let base = map(&[("d", Base::dir()), ("d/a", b(1, 1, 1, 1).unwrap())]);
        let remote = map(&[("d", Meta::dir()), ("d/a", F(1, 1)), ("d x", Meta::dir()), ("d.txt", F(1, 1))]);
        let local = map(&[("d x", Meta::dir()), ("d.txt", F(1, 1))]);
        let p = plan(&local, &remote, &base);
        assert_eq!(p, map(&[("d", DeleteRemote), ("d x", Record), ("d.txt", Record)]));
    }

    #[test]
    fn ignore_rules() {
        for p in ["~$doc.docx", "a/~$x.xlsx", "a/b.tmp", "X.TMP", ".DS_Store", "a/Thumbs.db", "f.pdf.nkpart", ".nextkloud-trash", ".nextkloud-trash/a.txt", "a/.~lock.x.odt#", "a/../b"] {
            assert!(ignored(p), "{p}");
        }
        for p in ["a.txt", ".obsidian/app.json", "a/.nextkloud-trash/x", "tmp/a", "a.tmpl"] {
            assert!(!ignored(p), "{p}");
        }
    }

    #[test]
    fn conflict_names() {
        assert_eq!(conflict_name("a/b/report.docx", "pc", "2026-11-26 1530"), "a/b/report (conflict pc 2026-11-26 1530).docx");
        assert_eq!(conflict_name("Makefile", "pc", "s"), "Makefile (conflict pc s)");
        assert_eq!(conflict_name(".bashrc", "pc", "s"), ".bashrc (conflict pc s)");
        assert_eq!(conflict_name("a.tar.gz", "pc", "s"), "a.tar (conflict pc s).gz");
    }

    #[test]
    fn stamps() {
        assert_eq!(stamp(0), "1970-01-01 0000");
        assert_eq!(stamp(1_795_707_000), "2026-11-26 1530");
        assert_eq!(stamp(951_782_400), "2000-02-29 0000");
    }
}
