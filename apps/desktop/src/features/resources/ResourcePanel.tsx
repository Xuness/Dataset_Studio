import { Button } from "@studio/ui";
import type { ModuleContext } from "@studio/ui";

/** Compatibility target for commands registered before settings became global. */
export default function ResourcePanel({ activateView }: ModuleContext) {
  return (
    <section className="content-bar">
      <span>缓存与性能选项已移至顶部“设置”菜单。</span>
      <Button onClick={() => activateView("core.resources")}>打开设置</Button>
    </section>
  );
}
