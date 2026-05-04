/** @type {import('tailwindcss').Config} */
function rgb(name) {
  return `rgb(var(${name}) / <alpha-value>)`;
}

export default {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        bg: {
          base: rgb("--bg-base"),
          surface: rgb("--bg-surface"),
          surface2: rgb("--bg-surface-2"),
          elevated: rgb("--bg-elevated"),
        },
        border: {
          DEFAULT: rgb("--border"),
          strong: rgb("--border-strong"),
        },
        text: {
          primary: rgb("--text-primary"),
          secondary: rgb("--text-secondary"),
          tertiary: rgb("--text-tertiary"),
        },
        accent: {
          DEFAULT: rgb("--accent"),
          2: rgb("--accent-2"),
        },
        success: rgb("--success"),
        warning: rgb("--warning"),
        danger: rgb("--danger"),
        info: rgb("--info"),
      },
      fontFamily: {
        sans: [
          "Inter",
          "Segoe UI Variable",
          "PingFang SC",
          "Noto Sans CJK SC",
          "system-ui",
          "sans-serif",
        ],
        mono: [
          "JetBrains Mono",
          "Cascadia Code",
          "Menlo",
          "ui-monospace",
          "monospace",
        ],
      },
      fontSize: {
        metric: ["2.25rem", { lineHeight: "1.1" }],
      },
      borderRadius: {
        card: "12px",
      },
      boxShadow: {
        elevated: "0 8px 24px rgba(0,0,0,0.45)",
      },
    },
  },
  plugins: [],
};
