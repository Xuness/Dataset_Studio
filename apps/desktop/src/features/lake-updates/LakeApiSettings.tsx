import { useState } from "react";
import { Button, ErrorDetails } from "@studio/ui";
import type { SettingsPageProps } from "../settings/types.js";
import { useLakeRefresh, useLakeStatus } from "./queries.js";
import { sites, dateLabel } from "./model.js";
import "./lake-updates.css";
import { LakeRelocationSettings } from "./LakeRelocationSettings.js";

export function LakeApiSettings({ client }: SettingsPageProps) {
  const status = useLakeStatus(client),
    refresh = useLakeRefresh(client);
  const [site, setSite] = useState<"danbooru" | "gelbooru">("danbooru");
  const [account, setAccount] = useState(""),
    [secret, setSecret] = useState("");
  const [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null),
    [notice, setNotice] = useState("");
  const [python, setPython] = useState<string | null>(null);
  const [stateRoot, setStateRoot] = useState("");
  const runtime = {
    python: python ?? status.data?.runtime.python ?? "",
    state_root: status.data?.runtime.state_root ?? stateRoot,
  };
  async function act(run: () => Promise<unknown>, notice: string) {
    setPending(true);
    setError(null);
    setNotice("");
    try {
      await run();
      setNotice(notice);
      await refresh();
    } catch (e) {
      setError(e);
      await refresh();
    } finally {
      setPending(false);
    }
  }
  return (
    <section className="lake-settings">
      <h2>数据湖 API</h2>
      <p>凭据在本机加密保存。连接检测会发出少量 API 请求。</p>
      {status.error && <ErrorDetails error={status.error} />}
      <>
        <div className="lake-fields">
          <h3>
            {status.data?.configured
              ? "修复或更换更新运行环境"
              : "配置更新运行环境"}
          </h3>
          <p className="lake-hint">
            更新服务由 Studio 内置提供。Python 环境需要安装项目声明的运行依赖。
          </p>
          <label>
            Python 可执行文件
            <input
              value={runtime.python}
              onChange={(e) => setPython(e.target.value)}
            />
          </label>
          <label>
            共享更新状态目录
            <input
              value={runtime.state_root}
              disabled={status.data?.configured}
              onChange={(e) => setStateRoot(e.target.value)}
            />
          </label>
          <Button
            disabled={pending || Object.values(runtime).some((v) => !v.trim())}
            onClick={() =>
              void act(
                () => client.lakeUpdates.configure(runtime),
                "运行环境验证通过，已保留任务和凭据",
              )
            }
          >
            验证并保存运行环境
          </Button>
          {status.data?.runtime.message && (
            <p role="status">
              {status.data.runtime.message}（{status.data.runtime.error_code}）
            </p>
          )}
          {status.data?.runtime.next_retry_ms && (
            <p className="lake-hint">
              下次自动重试：
              {dateLabel(
                new Date(status.data.runtime.next_retry_ms).toISOString(),
              )}
              ；也可立即重新验证。
            </p>
          )}
          {status.data?.configured && (
            <p className="lake-hint">更换解释器保留原状态目录、任务和凭据。</p>
          )}
        </div>
      </>
      {status.data?.configured && (
        <>
          <dl className="wb-property-list">
            <dt>更新服务</dt>
            <dd>Studio 内置</dd>
            <dt>后台运行器</dt>
            <dd>
              {status.data.worker_recent ? "近期心跳正常" : "尚无近期心跳"}
            </dd>
          </dl>
          <div className="lake-site-status">
            {(Object.keys(sites) as (keyof typeof sites)[]).map((name) => (
              <div key={name}>
                <span>{sites[name]}</span>
                <span>
                  {name === "yandere"
                    ? "匿名访问"
                    : status.data.credentials.find((c) => c.site === name)
                          ?.credential_set
                      ? "凭据已保存"
                      : "未配置凭据"}
                </span>
                <Button
                  disabled={pending}
                  onClick={() =>
                    void act(
                      async () => {
                        const r = await client.lakeUpdates.probe(name);
                        if (r.status !== 200)
                          throw new Error(`HTTP ${r.status}`);
                      },
                      `${sites[name]} 连接检测成功 · ${dateLabel(new Date().toISOString())}`,
                    )
                  }
                >
                  测试连接
                </Button>
              </div>
            ))}
          </div>
          <details open>
            <summary>保存或替换凭据</summary>
            <div className="lake-fields">
              <label>
                站点
                <select
                  aria-label="凭据站点"
                  value={site}
                  onChange={(e) => {
                    setSite(e.target.value as typeof site);
                    setSecret("");
                    setAccount("");
                  }}
                >
                  <option value="danbooru">Danbooru</option>
                  <option value="gelbooru">Gelbooru</option>
                </select>
              </label>
              <label>
                {site === "danbooru" ? "用户名" : "User ID"}
                <input
                  autoComplete="off"
                  value={account}
                  onChange={(e) => setAccount(e.target.value)}
                />
              </label>
              <label>
                API Key
                <input
                  type="password"
                  autoComplete="new-password"
                  value={secret}
                  onChange={(e) => setSecret(e.target.value)}
                  placeholder="已保存的密钥不会回显"
                />
              </label>
              <div className="lake-actions">
                <Button
                  disabled={pending || !account.trim() || !secret.trim()}
                  onClick={() =>
                    void act(async () => {
                      await client.lakeUpdates.setCredentials(
                        site === "danbooru"
                          ? { site, login: account.trim(), api_key: secret }
                          : { site, user_id: account.trim(), api_key: secret },
                      );
                      setSecret("");
                    }, "凭据已保存")
                  }
                >
                  保存凭据
                </Button>
                <Button
                  disabled={
                    pending ||
                    !status.data.credentials.find((c) => c.site === site)
                      ?.credential_set
                  }
                  onClick={() => {
                    if (
                      window.confirm(
                        `清除 ${sites[site]} 的已保存凭据？现有任务会保留。`,
                      )
                    )
                      void act(
                        () => client.lakeUpdates.clearCredentials(site),
                        "凭据已清除",
                      );
                  }}
                >
                  清除已保存凭据
                </Button>
              </div>
            </div>
          </details>
          <p className="lake-hint">
            关闭项目或数据湖标签不会停止更新。退出托管引擎会停止运行器，检查点保留供下次恢复；此处未安装系统定时服务。
          </p>
        </>
      )}
      {error != null && <ErrorDetails error={error} />}
      {status.data?.configured && <LakeRelocationSettings client={client} />}
      {notice && <p role="status">{notice}</p>}
    </section>
  );
}
