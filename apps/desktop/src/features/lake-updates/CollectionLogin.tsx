import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { StudioError, type StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { Button, ErrorDetails } from "@studio/ui";
import { lakeKey } from "./queries.js";

export function CollectionLogin({
  client,
  account,
  label,
  disabled,
  onActive,
  onSaved,
}: {
  client: StudioClient;
  account: Schema["CollectionAccount"] | undefined;
  label: string;
  disabled: boolean;
  onActive: (active: boolean) => void;
  onSaved: (account: Schema["CollectionAccount"]) => Promise<void>;
}) {
  const assistant = client.collectionLogin;
  const status = useQuery({
    queryKey: [...lakeKey(client), "login-assistant"],
    queryFn: () => assistant!.status(),
    enabled: !!assistant,
    refetchInterval: 1500,
    retry: false,
  });
  const session = status.data?.session;
  const active =
    !!session &&
    ["waiting", "verifying", "unconfirmed"].includes(session.phase);
  const [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null);
  const startIntent = useRef<Schema["StartCollectionLogin"] | null>(null);
  const delivered = useRef<string | null>(null);
  const observedActive = useRef<string | null>(null);
  useEffect(() => {
    if (active && session) observedActive.current = session.id;
    onActive(active || pending);
  }, [active, pending, onActive, session]);
  useEffect(() => {
    if (
      session?.phase === "succeeded" &&
      session.result &&
      observedActive.current === session.id &&
      delivered.current !== session.id
    ) {
      delivered.current = session.id;
      void onSaved(session.result.account).catch(setError);
    }
  }, [session, onSaved]);
  async function act(run: () => Promise<unknown>) {
    setPending(true);
    setError(null);
    try {
      await run();
    } catch (e) {
      setError(e);
    } finally {
      await status.refetch();
      setPending(false);
    }
  }
  const confirming =
    session?.phase === "verifying" || session?.phase === "unconfirmed";
  const visibleError =
    error ||
    (session?.error
      ? new StudioError(session.error.code, session.error.message)
      : null);
  return (
    <section
      className="collection-login-assistant"
      aria-label="Pixiv 浏览器登录助手"
    >
      <p>
        在独立窗口完成 Pixiv 登录，再返回这里验证并保存。窗口有效期 30
        分钟，验证成功后自动关闭。
      </p>
      {!assistant ? (
        <p className="lake-hint">
          浏览器登录助手在 Windows 桌面版中提供；当前可使用下方的 Cookie 导入。
        </p>
      ) : (
        <>
          <div className="lake-actions">
            <Button
              disabled={
                disabled ||
                pending ||
                active ||
                !label.trim() ||
                !status.data?.available
              }
              onClick={() =>
                void act(async () => {
                  const input = {
                    account_id: account?.id ?? crypto.randomUUID(),
                    expected_revision: account?.revision ?? null,
                    label: label.trim(),
                  };
                  const previous = startIntent.current;
                  if (
                    !previous ||
                    (account
                      ? previous.account_id !== account.id
                      : previous.expected_revision !== null) ||
                    previous.label !== input.label ||
                    previous.expected_revision !== input.expected_revision ||
                    (session && delivered.current === session.id) ||
                    (session &&
                      ["cancelled", "expired"].includes(session.phase))
                  )
                    startIntent.current = {
                      ...input,
                      request_key: crypto.randomUUID(),
                    };
                  await assistant.start(startIntent.current!);
                })
              }
            >
              通过浏览器登录
            </Button>
            {active && (
              <>
                <Button
                  disabled={pending || !session.window_open}
                  onClick={() => void act(() => assistant.show(session.id))}
                >
                  返回登录窗口
                </Button>
                <Button
                  disabled={pending || session.phase === "verifying"}
                  onClick={() => void act(() => assistant.finish(session.id))}
                >
                  {session.phase === "unconfirmed"
                    ? "重新确认保存结果"
                    : session.phase === "verifying" || pending
                      ? "正在处理…"
                      : "已登录，验证并保存"}
                </Button>
                <Button
                  disabled={pending || confirming}
                  onClick={() => void act(() => assistant.cancel(session.id))}
                >
                  取消此次登录
                </Button>
              </>
            )}
          </div>
          {active && (
            <p role="status">
              正在为“{session.label}”登录。
              {session.phase === "unconfirmed"
                ? "连接曾中断，请重新确认保存结果；会继续使用同一次请求。"
                : "请在官方网站完成登录及可能的验证码。账号密码直接输入 Pixiv 页面。"}
            </p>
          )}
          {session?.phase === "succeeded" && (
            <p role="status">
              上次浏览器登录已验证 · 用户{" "}
              {session.result?.account.bound_user_id ?? "已确认"}。分级及 AI
              显示条件仍需分别验证。
            </p>
          )}
          {session?.phase === "cancelled" && (
            <p role="status">登录窗口已关闭，可重新开始登录。</p>
          )}
          {session?.phase === "expired" && (
            <p role="status">此次登录窗口已超时，请重新开始。</p>
          )}
          {status.error && <ErrorDetails error={status.error} />}
        </>
      )}
      {visibleError != null && <ErrorDetails error={visibleError} />}
    </section>
  );
}
