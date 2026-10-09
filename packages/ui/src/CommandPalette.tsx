import { useEffect, useId, useMemo, useRef, useState } from "react";
import { Check, Search } from "lucide-react";

export type Command = {
  id: string;
  label: string;
  /** Menu or category shown on the right, also searchable. */
  group: string;
  shortcut?: string;
  checked?: boolean;
  disabled?: boolean;
  run: () => void;
};

/** Every query token must appear in order; earlier label hits rank first. */
function score(command: Command, tokens: string[]) {
  const label = command.label.toLowerCase();
  const text = command.group.toLowerCase() + " " + label;
  let total = 0;
  for (const token of tokens) {
    const direct = label.indexOf(token);
    if (direct >= 0) {
      total += direct === 0 ? 0 : 1 + direct / 100;
      continue;
    }
    if (text.includes(token)) {
      total += 3;
      continue;
    }
    let at = 0;
    for (const char of token) {
      at = text.indexOf(char, at);
      if (at < 0) return null;
      at += 1;
    }
    total += 6;
  }
  return total + (command.disabled ? 10 : 0);
}

/** Ctrl+P command search over the editor's menus and quick destinations. */
export function CommandPalette({
  commands,
  onClose,
}: {
  commands: Command[];
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const id = useId();
  useEffect(() => {
    const previous = document.activeElement;
    const node = dialog.current;
    node?.showModal();
    return () => {
      node?.close();
      if (previous instanceof HTMLElement && previous.isConnected)
        previous.focus();
    };
  }, []);
  const results = useMemo(() => {
    const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (!tokens.length) return commands;
    return commands
      .map((command, index) => ({
        command,
        index,
        rank: score(command, tokens),
      }))
      .filter((item) => item.rank !== null)
      .sort((a, b) => a.rank! - b.rank! || a.index - b.index)
      .map((item) => item.command);
  }, [commands, query]);
  const current = Math.min(active, Math.max(0, results.length - 1));
  useEffect(() => {
    list.current
      ?.querySelector(`[data-index="${current}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [current]);
  function run(command: Command | undefined) {
    if (!command || command.disabled) return;
    onClose();
    // Let the palette release focus before the command moves it.
    requestAnimationFrame(command.run);
  }
  return (
    <dialog
      ref={dialog}
      className="command-palette"
      aria-label="命令搜索"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="command-search">
        <Search size={14} />
        <input
          autoFocus
          role="combobox"
          aria-expanded="true"
          aria-controls={id}
          aria-activedescendant={
            results.length ? `${id}-${current}` : undefined
          }
          aria-label="搜索命令"
          placeholder="搜索命令、面板、工作集或数据湖…"
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setActive(0);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              const step = event.key === "ArrowDown" ? 1 : -1;
              setActive(
                (current + step + results.length) % Math.max(1, results.length),
              );
            } else if (event.key === "Enter") {
              event.preventDefault();
              run(results[current]);
            }
          }}
        />
      </div>
      <ul ref={list} id={id} role="listbox" className="command-list">
        {results.map((command, index) => (
          <li
            key={command.id}
            id={`${id}-${index}`}
            data-index={index}
            role="option"
            aria-selected={index === current}
            aria-disabled={command.disabled || undefined}
            onMouseMove={() => setActive(index)}
            onClick={() => run(command)}
          >
            <span className="menu-check">
              {command.checked && <Check size={12} />}
            </span>
            <span className="command-label">{command.label}</span>
            <span className="command-group">{command.group}</span>
            {command.shortcut && (
              <kbd className="menu-shortcut">{command.shortcut}</kbd>
            )}
          </li>
        ))}
        {!results.length && <li className="command-empty">没有匹配的命令</li>}
      </ul>
    </dialog>
  );
}
