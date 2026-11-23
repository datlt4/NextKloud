import { For, Show, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { api } from "./api";
import { fmtNum, t } from "./i18n";
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
      <p class="err pin-err">{props.busy ? t("sc.checking") : props.error ?? ""}</p>
      <div class="pin-keys">
        <For each={["1", "2", "3", "4", "5", "6", "7", "8", "9"]}>{(d) => <button onClick={() => push(d)}>{d}</button>}</For>
        <span />
        <button onClick={() => push("0")}>0</button>
        <button onClick={back} title={t("c.delete")}><Icon name="back" /></button>
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
      <h3>{t("sc.recTitle")}</h3>
      <p class="muted sm" innerHTML={t("sc.recDesc")} />
      <code class="rec-code">{props.code}</code>
      <button class="btn ghost" onClick={async () => {
        try {
          await navigator.clipboard.writeText(props.code);
          setCopied(true);
        } catch {}
      }}>{copied() ? t("sc.copiedCode") : t("sc.copyCode")}</button>
      <label class="rec-ok"><input type="checkbox" checked={saved()} onChange={(e) => setSaved(e.currentTarget.checked)} /> {t("sc.savedRec")}</label>
      <button class="btn primary block" disabled={!saved()} onClick={props.onDone}>{t("sc.continue")}</button>
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
        <Show when={!st.loading} fallback={<p class="muted">{t("c.loading")}</p>}>
          <Show when={(st()?.wait ?? 0) > 0}>
            <p class="err">{t("sc.lockedOut", { m: Math.ceil(st()!.wait / 60) })}</p>
          </Show>
          {(() => {
            const x = s();
            switch (x.k) {
              case "pin":
                return (
                  <>
                    <PinPad title={t("sc.enterPin")} busy={busy()} error={err()} onDone={(pin) => run(async () => {
                      await api.secretUnlock({ pin });
                      props.onUnlocked();
                    }).then(() => refetch())} />
                    <button class="btn link" onClick={() => { setErr(""); setStep({ k: "rec" }); }}>{t("sc.forgotPin")}</button>
                  </>
                );
              case "new1":
                return <PinPad title={t("sc.setupTitle")} hint={t("sc.setupHint")} error={err()} onDone={(p) => { setErr(""); setStep({ k: "new2", first: p }); }} />;
              case "new2":
                return <PinPad title={t("sc.rePin")} busy={busy()} error={err()} onDone={(p) => {
                  if (p !== x.first) {
                    setErr(t("sc.mismatch"));
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
                    <h3>{t("sc.unlockRec")}</h3>
                    <input class="rec-in" value={code()} onInput={(e) => setCode(e.currentTarget.value)} placeholder="XXXX-XXXX-XXXX-XXXX-XXXX-XXXX" autocomplete="off" />
                    <p class="err pin-err">{busy() ? t("sc.checking") : err()}</p>
                    <button class="btn primary block" disabled={busy() || code().replace(/[^a-z0-9]/gi, "").length < 24}>{t("sc.unlock")}</button>
                    <button type="button" class="btn link" onClick={() => { setErr(""); setStep({ k: "pin" }); }}>{t("sc.backPin")}</button>
                  </form>
                );
              case "reset1":
                return <PinPad title={t("sc.newPin")} error={err()} onDone={(p) => { setErr(""); setStep({ k: "reset2", first: p }); }} />;
              case "reset2":
                return <PinPad title={t("sc.reNewPin")} busy={busy()} error={err()} onDone={(p) => {
                  if (p !== x.first) {
                    setErr(t("sc.mismatch"));
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
          <h3><Icon name="lock" /> {t("f.secretSettings")}</h3>
          <button type="button" class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <Show when={msg()}><p class="ok-msg">{msg()}</p></Show>
        <Show when={err()}><p class="err">{err()}</p></Show>

        <label>{t("sc.changePin")}</label>
        <div class="two">
          {pinIn(oldPin, setOldPin, t("sc.curPin"))}
          {pinIn(newPin, setNewPin, t("sc.newPin2"))}
        </div>
        <button class="btn ghost" disabled={busy() || !six(oldPin()) || !six(newPin())} onClick={() => run(async () => {
          await api.secretChangePin(oldPin(), newPin());
          setOldPin("");
          setNewPin("");
          setMsg(t("sc.pinChanged"));
        })}>{t("sc.changePin")}</button>

        <label>{t("sc.newRecTitle")}</label>
        <Show when={code()} fallback={
          <div class="two">
            {pinIn(pin2, setPin2, t("sc.curPin"))}
            <button class="btn ghost" disabled={busy() || !six(pin2())} onClick={() => run(async () => {
              setCode(await api.secretNewRecovery(pin2()));
              setPin2("");
            })}>{t("sc.makeRec")}</button>
          </div>
        }>
          <code class="rec-code">{code()}</code>
          <p class="muted sm">{t("sc.saveRecWarn")}</p>
        </Show>

        <label class="danger-txt">{t("sc.delVault")}</label>
        <div class="two">
          {pinIn(delPin, setDelPin, t("sc.curPin"))}
          <input placeholder={t("sc.typeXoa")} value={confirmTxt()} onInput={(e) => setConfirmTxt(e.currentTarget.value)} />
        </div>
        <button class="btn danger" disabled={busy() || !six(delPin()) || confirmTxt().trim().toUpperCase() !== "XOA"} onClick={() => run(async () => {
          await api.secretDestroy(delPin());
          props.onDestroyed();
        })}>{t("c.delForever")}</button>
      </div>
    </div>
  );
}
