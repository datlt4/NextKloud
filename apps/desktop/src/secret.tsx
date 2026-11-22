import { For, Show, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { api } from "./api";
import { errText } from "./fileops";
import { Icon } from "./icons";

/** Bàn phím PIN 6 số: bấm nút hoặc gõ phím số, đủ 6 số thì tự gửi. */
function PinPad(props: { title: string; hint?: string; busy?: boolean; error?: string; onDone: (pin: string) => void }) {
  const [pin, setPin] = createSignal("");
  const push = (d: string) => {
    if (props.busy) return;
    const n = (pin() + d).slice(0, 6);
    setPin(n);
    if (n.length === 6) {
      props.onDone(n);
      setTimeout(() => setPin(""), 150);
    }
  };
  const back = () => setPin((p) => p.slice(0, -1));
  onMount(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.target as Element).closest?.("input, textarea")) return;
      if (/^[0-9]$/.test(e.key)) push(e.key);
      else if (e.key === "Backspace") back();
    };
    document.addEventListener("keydown", key);
    onCleanup(() => document.removeEventListener("keydown", key));
  });
  return (
    <div class="pin">
      <h3>{props.title}</h3>
      <Show when={props.hint}><p class="muted sm">{props.hint}</p></Show>
      <div class="pin-dots" classList={{ shake: !!props.error }}>
        <For each={[0, 1, 2, 3, 4, 5]}>{(i) => <i classList={{ on: pin().length > i }} />}</For>
      </div>
      <p class="err pin-err">{props.busy ? "Đang kiểm tra…" : props.error ?? ""}</p>
      <div class="pin-keys">
        <For each={["1", "2", "3", "4", "5", "6", "7", "8", "9"]}>{(d) => <button onClick={() => push(d)}>{d}</button>}</For>
        <span />
        <button onClick={() => push("0")}>0</button>
        <button onClick={back} title="Xoá"><Icon name="back" /></button>
      </div>
    </div>
  );
}

/** Hiện mã khôi phục 1 lần, bắt tick "đã lưu" mới cho đi tiếp. */
function RecoveryShow(props: { code: string; onDone: () => void }) {
  const [saved, setSaved] = createSignal(false);
  const [copied, setCopied] = createSignal(false);
  return (
    <div class="pin">
      <h3>Mã khôi phục</h3>
      <p class="muted sm">Dùng khi quên PIN. Chỉ hiện <b>một lần</b>. Lưu vào trình quản lý mật khẩu hoặc giấy. Mất cả PIN lẫn mã này thì <b>không ai mở được</b> dữ liệu, kể cả admin.</p>
      <code class="rec-code">{props.code}</code>
      <button class="btn ghost" onClick={async () => {
        try {
          await navigator.clipboard.writeText(props.code);
          setCopied(true);
        } catch {}
      }}>{copied() ? "Đã chép ✓" : "Chép mã"}</button>
      <label class="rec-ok"><input type="checkbox" checked={saved()} onChange={(e) => setSaved(e.currentTarget.checked)} /> Tôi đã lưu mã khôi phục</label>
      <button class="btn primary block" disabled={!saved()} onClick={props.onDone}>Tiếp tục</button>
    </div>
  );
}

/** Cổng vào khu Bí mật: tạo lần đầu (PIN + mã khôi phục), mở bằng PIN, hoặc mã khôi phục rồi đặt PIN mới. */
export function SecretGate(props: { onUnlocked: () => void }) {
  const [st, { refetch }] = createResource(() => api.secretStatus());
  type Step = { k: "pin" } | { k: "new1" } | { k: "new2"; first: string } | { k: "rec" } | { k: "code"; code: string } | { k: "reset1" } | { k: "reset2"; first: string };
  const [step, setStep] = createSignal<Step>({ k: "pin" });
  const [err, setErr] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [code, setCode] = createSignal("");
  const run = async (f: () => Promise<void>) => {
    setBusy(true);
    setErr("");
    try {
      await f();
    } catch (e) {
      setErr(errText(e));
    } finally {
      setBusy(false);
    }
  };
  const s = () => {
    const x = step();
    return st()?.exists === false && x.k === "pin" ? ({ k: "new1" } as Step) : x;
  };
  return (
    <div class="secret-gate">
      <div class="gate-box glass">
        <div class="gate-ic"><Icon name="lock" size={30} /></div>
        <Show when={!st.loading} fallback={<p class="muted">Đang tải…</p>}>
          <Show when={(st()?.wait ?? 0) > 0}>
            <p class="err">Sai quá nhiều lần, thử lại sau {Math.ceil(st()!.wait / 60)} phút.</p>
          </Show>
          {(() => {
            const x = s();
            switch (x.k) {
              case "pin":
                return (
                  <>
                    <PinPad title="Nhập PIN khu Bí mật" busy={busy()} error={err()} onDone={(pin) => run(async () => {
                      await api.secretUnlock({ pin });
                      props.onUnlocked();
                    }).then(() => refetch())} />
                    <button class="btn link" onClick={() => { setErr(""); setStep({ k: "rec" }); }}>Quên PIN? Dùng mã khôi phục</button>
                  </>
                );
              case "new1":
                return <PinPad title="Tạo khu Bí mật" hint="Đặt PIN 6 số. File trong khu này được mã hoá, tự khoá sau 10 phút không dùng." error={err()} onDone={(p) => { setErr(""); setStep({ k: "new2", first: p }); }} />;
              case "new2":
                return <PinPad title="Nhập lại PIN" busy={busy()} error={err()} onDone={(p) => {
                  if (p !== x.first) {
                    setErr("PIN không khớp, đặt lại");
                    setStep({ k: "new1" });
                    return;
                  }
                  run(async () => {
                    setStep({ k: "code", code: await api.secretSetup(p) });
                  });
                }} />;
              case "code":
                return <RecoveryShow code={x.code} onDone={props.onUnlocked} />;
              case "rec":
                return (
                  <form class="pin" onSubmit={(e) => {
                    e.preventDefault();
                    run(async () => {
                      await api.secretUnlock({ recovery: code() });
                      setCode("");
                      setStep({ k: "reset1" });
                    });
                  }}>
                    <h3>Mở bằng mã khôi phục</h3>
                    <input class="rec-in" value={code()} onInput={(e) => setCode(e.currentTarget.value)} placeholder="XXXX-XXXX-XXXX-XXXX-XXXX-XXXX" autocomplete="off" />
                    <p class="err pin-err">{busy() ? "Đang kiểm tra…" : err()}</p>
                    <button class="btn primary block" disabled={busy() || code().replace(/[^a-z0-9]/gi, "").length < 24}>Mở khoá</button>
                    <button type="button" class="btn link" onClick={() => { setErr(""); setStep({ k: "pin" }); }}>Quay lại nhập PIN</button>
                  </form>
                );
              case "reset1":
                return <PinPad title="Đặt PIN mới" error={err()} onDone={(p) => { setErr(""); setStep({ k: "reset2", first: p }); }} />;
              case "reset2":
                return <PinPad title="Nhập lại PIN mới" busy={busy()} error={err()} onDone={(p) => {
                  if (p !== x.first) {
                    setErr("PIN không khớp, đặt lại");
                    setStep({ k: "reset1" });
                    return;
                  }
                  run(async () => {
                    await api.secretChangePin(null, p);
                    props.onUnlocked();
                  });
                }} />;
            }
          })()}
        </Show>
      </div>
    </div>
  );
}

/** Đổi PIN, tạo mã khôi phục mới, xoá vault. */
export function SecretSettings(props: { onClose: () => void; onDestroyed: () => void }) {
  const [oldPin, setOldPin] = createSignal("");
  const [newPin, setNewPin] = createSignal("");
  const [pin2, setPin2] = createSignal("");
  const [delPin, setDelPin] = createSignal("");
  const [confirmTxt, setConfirmTxt] = createSignal("");
  const [code, setCode] = createSignal("");
  const [msg, setMsg] = createSignal("");
  const [err, setErr] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const run = async (f: () => Promise<void>) => {
    setBusy(true);
    setErr("");
    setMsg("");
    try {
      await f();
    } catch (e) {
      setErr(errText(e));
    } finally {
      setBusy(false);
    }
  };
  const six = (v: string) => /^\d{6}$/.test(v);
  const pinIn = (v: () => string, set: (s: string) => void, ph: string) => (
    <input type="password" inputmode="numeric" maxlength={6} autocomplete="off" placeholder={ph} value={v()} onInput={(e) => set(e.currentTarget.value.replace(/\D/g, ""))} />
  );
  return (
    <div class="modal-bg" onClick={() => !busy() && props.onClose()}>
      <div class="modal glass form secret-set" onClick={(e) => e.stopPropagation()}>
        <div class="modal-head">
          <h3><Icon name="lock" /> Cài đặt Bí mật</h3>
          <button type="button" class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <Show when={msg()}><p class="ok-msg">{msg()}</p></Show>
        <Show when={err()}><p class="err">{err()}</p></Show>

        <label>Đổi PIN</label>
        <div class="two">
          {pinIn(oldPin, setOldPin, "PIN hiện tại")}
          {pinIn(newPin, setNewPin, "PIN mới")}
        </div>
        <button class="btn ghost" disabled={busy() || !six(oldPin()) || !six(newPin())} onClick={() => run(async () => {
          await api.secretChangePin(oldPin(), newPin());
          setOldPin("");
          setNewPin("");
          setMsg("Đã đổi PIN");
        })}>Đổi PIN</button>

        <label>Mã khôi phục mới (mã cũ hết hiệu lực)</label>
        <Show when={code()} fallback={
          <div class="two">
            {pinIn(pin2, setPin2, "PIN")}
            <button class="btn ghost" disabled={busy() || !six(pin2())} onClick={() => run(async () => {
              setCode(await api.secretNewRecovery(pin2()));
              setPin2("");
            })}>Tạo mã mới</button>
          </div>
        }>
          <code class="rec-code">{code()}</code>
          <p class="muted sm">Lưu mã này ngay, đóng hộp thoại là không xem lại được.</p>
        </Show>

        <label class="danger-txt">Xoá khu Bí mật (xoá vĩnh viễn mọi file trong đó)</label>
        <div class="two">
          {pinIn(delPin, setDelPin, "PIN")}
          <input placeholder='Gõ "XOA" để xác nhận' value={confirmTxt()} onInput={(e) => setConfirmTxt(e.currentTarget.value)} />
        </div>
        <button class="btn danger" disabled={busy() || !six(delPin()) || confirmTxt().trim().toUpperCase() !== "XOA"} onClick={() => run(async () => {
          await api.secretDestroy(delPin());
          props.onDestroyed();
        })}>Xoá vĩnh viễn</button>
      </div>
    </div>
  );
}
