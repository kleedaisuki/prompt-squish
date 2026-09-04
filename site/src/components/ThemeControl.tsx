import { useEffect, useState } from "react";

type Theme = "auto" | "light" | "dark";

type Props = {
  labels: Record<Theme, string> & { label: string };
};

const themes: Theme[] = ["auto", "light", "dark"];

function applyTheme(theme: Theme) {
  document.documentElement.dataset.moeTheme = theme;
  localStorage.setItem("xmlsquish-theme", theme);
}

export default function ThemeControl({ labels }: Props) {
  const [theme, setTheme] = useState<Theme>("auto");

  useEffect(() => {
    const saved = localStorage.getItem("xmlsquish-theme");
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
          <span aria-hidden="true">{item === "auto" ? "◐" : item === "light" ? "☀" : "☾"}</span>
          <span className="moe-visually-hidden">{labels[item]}</span>
        </button>
      ))}
    </div>
  );
}
