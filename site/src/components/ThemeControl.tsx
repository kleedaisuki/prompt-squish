import { useEffect, useState } from "react";
import "./ThemeControl.css";

type Theme = "auto" | "light" | "dark";

type Props = {
  labels: Record<Theme, string> & { label: string };
};

const themes: Theme[] = ["auto", "light", "dark"];

function applyTheme(theme: Theme) {
  document.documentElement.dataset.moeTheme = theme;
  try {
    localStorage.setItem("xmlsquish-theme", theme);
  } catch {
    /* Theme switching must still work when storage is unavailable. */
  }
}

export default function ThemeControl({ labels }: Props) {
  const [theme, setTheme] = useState<Theme>("auto");

  useEffect(() => {
    let saved: string | null = null;
    try {
      saved = localStorage.getItem("xmlsquish-theme");
    } catch {
      /* Keep the system preference without persistent storage. */
    }
    if (saved === "light" || saved === "dark" || saved === "auto") {
      setTheme(saved);
      applyTheme(saved);
    }
  }, []);

  return (
    <div className="theme-control" role="group" aria-label={labels.label}>
      {themes.map((item) => (
        <button
          className="theme-control__option"
          type="button"
          aria-pressed={theme === item}
          title={`${labels.label}: ${labels[item]}`}
          onClick={() => {
            setTheme(item);
            applyTheme(item);
          }}
          key={item}
        >
          <span aria-hidden="true">
            {item === "auto" ? "◐" : item === "light" ? "☀" : "☾"}
          </span>
          <span className="moe-visually-hidden">{labels[item]}</span>
        </button>
      ))}
    </div>
  );
}
