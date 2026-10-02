import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { Button, ErrorDetails } from "@studio/ui";
import { lakeKey, useLakeRefresh } from "./queries.js";
import { dateLabel } from "./model.js";

const accountStates: Record<string, string> = {
  valid: "已验证",
  unverified: "待验证",
  expired: "已失效",
  challenge: "需要交互验证",
  cleared: "凭据已清除",
};
export function CollectionApiSettings({ client }: { client: StudioClient }) {
  const refresh = useLakeRefresh(client);
  const [cursor, setCursor] = useState<string | undefined>(undefined);
  const accounts = useQuery({
    queryKey: [...lakeKey(client), "collection-accounts", cursor],
    queryFn: ({ signal }) =>
      client.sourceCollections.accounts({ signal, limit: 200, cursor }),
  });
  const [id, setId] = useState(""),
    [label, setLabel] = useState("Pixiv 登录会话"),
    [secret, setSecret] = useState("");
  const [imported, setImported] = useState<Schema["CollectionCookie"][] | null>(
    null,
  );
  const [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null),
    [notice, setNotice] = useState("");
  const selected = accounts.data?.items.find((a) => a.id === id);
  async function act(run: () => Promise<unknown>, message: string) {
    setPending(true);
    setError(null);
    setNotice("");
    try {
      await run();
      setNotice(message);
      await refresh();
    } catch (e) {
      setError(e);
      await refresh();
    } finally {
      setPending(false);
    }
  }
  async function load(file: File | undefined) {
    setError(null);
    try {
      if (!file) return;
      if (file.size > 65536)
        throw new Error("Cookie JSON 文件不能超过 64 KiB。");
      const raw = JSON.parse(await file.text()) as unknown;
      const items = Array.isArray(raw)
        ? raw
        : raw && typeof raw === "object" && "cookies" in raw
          ? raw.cookies
          : null;
      if (!Array.isArray(items))
        throw new Error("文件需要包含 Cookie 数组或 cookies 数组字段。");
      const cookies = items
        .filter(
          (v) =>
            v &&
            typeof v === "object" &&
            [
              "pixiv.net",
              ".pixiv.net",
              "www.pixiv.net",
              "accounts.pixiv.net",
            ].includes(v.domain),
        )
        .map((v) => ({
          name: String(v.name),
          value: String(v.value),
          domain: String(v.domain),
          path: v.path || "/",
          secure: true,
          http_only: Boolean(v.http_only ?? v.httpOnly),
          expires_unix:
            v.expires_unix ??
            (v.expirationDate > 0 ? Math.floor(v.expirationDate) : null),
        }));
      if (!cookies.length) throw new Error("文件中没有可用的 Pixiv Cookie。");
      setImported(cookies);
      setSecret("");
      setNotice(`已读取 ${cookies.length} 项 Pixiv Cookie，尚未保存。`);
    } catch {
      setImported(null);
      setError(
        new Error("Cookie 文件格式不符合要求；请提供 Pixiv Cookie JSON 数组。"),
      );
    }
  }
  return (
    <details className="collection-api-settings" open>
      <summary>Pixiv</summary>
      <p>默认公开访问，无需填写凭据。任务表单可选择这里保存的登录会话。</p>
      {accounts.error && <ErrorDetails error={accounts.error} />}
      <div className="lake-fields">
        <label>
          登录会话
          <select
            aria-label="Pixiv 登录会话"
            value={id}
            onChange={(e) => {
              setId(e.target.value);
              setSecret("");
              setImported(null);
              const account = accounts.data?.items.find(
                (a) => a.id === e.target.value,
              );
              setLabel(account?.label ?? "Pixiv 登录会话");
              setNotice("");
            }}
          >
            <option value="">新增登录会话</option>
            {accounts.data?.items
              .filter((a) => a.mode === "session")
              .map((a) => (
                <option key={a.id} value={a.id}>
                  {a.label} · {accountStates[a.state] ?? a.state}
                </option>
              ))}
          </select>
        </label>
        {selected && (
          <p className="lake-hint">
            {accountStates[selected.state] ?? selected.state} · 绑定用户{" "}
            {selected.bound_user_id ?? "尚未确认"} · 上次验证{" "}
            {dateLabel(selected.last_probe_at)}
          </p>
        )}
        <label>
          名称
          <input
            maxLength={80}
            value={label}
            onChange={(e) => setLabel(e.target.value)}
          />
        </label>
        <label>
          PHPSESSID
          <input
            aria-label="Pixiv PHPSESSID"
            type="password"
            autoComplete="new-password"
            value={secret}
            onChange={(e) => {
              setSecret(e.target.value);
              setImported(null);
            }}
            placeholder="粘贴 Cookie 值；已保存的值不会回显"
          />
        </label>
        <label>
          或导入 Pixiv Cookie JSON
          <input
            aria-label="导入 Pixiv Cookie JSON"
            type="file"
            accept=".json,application/json"
            onChange={(e) => {
              void load(e.target.files?.[0]);
              e.target.value = "";
            }}
          />
        </label>
        <div className="lake-actions">
          <Button
            disabled={pending || !label.trim() || (!secret.trim() && !imported)}
            onClick={() =>
              void act(async () => {
                const account = await client.sourceCollections.saveAccount({
                  account_id: id || crypto.randomUUID(),
                  request_key: crypto.randomUUID(),
                  expected_revision: selected?.revision ?? null,
                  mode: "session",
                  label: label.trim(),
                  cookies: imported ?? [
                    {
                      name: "PHPSESSID",
                      value: secret
                        .trim()
                        .replace(/^PHPSESSID=/, "")
                        .replace(/;$/, ""),
                      domain: ".pixiv.net",
                      path: "/",
                      secure: true,
                      http_only: true,
                      expires_unix: null,
                    },
                  ],
                });
                setId(account.id);
                setSecret("");
                setImported(null);
              }, "凭据已加密保存，请验证会话后用于采集。")
            }
          >
            保存 Pixiv 凭据
          </Button>
          <Button
            disabled={pending || !selected?.credential_set}
            onClick={() =>
              void act(async () => {
                await client.sourceCollections.probeAccount(selected!.id, {
                  request_key: crypto.randomUUID(),
                  expected_revision: selected!.revision,
                });
              }, "服务端已确认登录身份。分级和 AI 显示条件仍单独记录为未知。")
            }
          >
            验证 Pixiv 会话
          </Button>
          <Button
            disabled={pending || !selected?.credential_set}
            onClick={() =>
              void act(async () => {
                await client.sourceCollections.clearAccount(selected!.id, {
                  request_key: crypto.randomUUID(),
                  expected_revision: selected!.revision,
                });
                setSecret("");
                setImported(null);
              }, "已清除该会话的本机凭据，采集记录保留。")
            }
          >
            清除 Pixiv 凭据
          </Button>
        </div>
        {accounts.data?.next_cursor && (
          <Button
            onClick={() => {
              setCursor(accounts.data!.next_cursor!);
              setId("");
            }}
          >
            后续会话
          </Button>
        )}
        {cursor && (
          <Button
            onClick={() => {
              setCursor(undefined);
              setId("");
            }}
          >
            首批会话
          </Button>
        )}
      </div>
      {error != null && <ErrorDetails error={error} />}
      {notice && <p role="status">{notice}</p>}
    </details>
  );
}
