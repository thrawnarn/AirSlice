import { useState, type FormEvent } from "react";
import { api } from "./api";
import { Notice } from "./ui";
import { tr } from "./i18n";

export function Login({ setup, setupBlocked, onDone }: { setup: boolean; setupBlocked: boolean; onDone: () => void }) {
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    if (setup && password !== confirm) {
      setError(tr("The passwords don't match."));
      return;
    }
    setBusy(true);
    try {
      await api.post(setup ? "auth/setup" : "auth/login", { password });
      onDone();
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy(false);
    }
  }

  if (setup && setupBlocked) {
    return (
      <main className="login">
        <div className="login-box">
          <img className="login-mark" src="./icon-192.png" alt="" />
          <h1 className="page-title">{tr("Set up AirSlice")}</h1>
          <p className="muted">
            {tr("AirSlice has no password yet. Set one in the AirSlice panel in Home Assistant (Settings, Access) or in the add-on's options, then sign in here.")}
          </p>
          <button className="btn primary block" onClick={onDone}>
            {tr("Check again")}
          </button>
        </div>
      </main>
    );
  }

  return (
    <main className="login">
      <form className="login-box" onSubmit={submit}>
        <img className="login-mark" src="./icon-192.png" alt="" />
        <h1 className="page-title">{setup ? tr("Set up AirSlice") : "AirSlice"}</h1>
        <p className="muted">
          {setup
            ? tr("Choose a password for this server. You'll stay signed in on this device.")
            : tr("Sign in to send models to your printer.")}
        </p>
        <label className="field">
          <span>{tr("Password")}</span>
          <input
            className="input"
            type="password"
            autoComplete={setup ? "new-password" : "current-password"}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoFocus
            required
            minLength={setup ? 8 : undefined}
          />
          {setup && <small>{tr("At least 8 characters.")}</small>}
        </label>
        {setup && (
          <label className="field">
            <span>{tr("Repeat password")}</span>
            <input
              className="input"
              type="password"
              autoComplete="new-password"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              required
            />
          </label>
        )}
        {error && <Notice tone="err">{error}</Notice>}
        <button className="btn primary block" disabled={busy}>
          {setup ? tr("Save password") : tr("Sign in")}
        </button>
      </form>
    </main>
  );
}
