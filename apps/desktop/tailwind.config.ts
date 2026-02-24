import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        canvas: "#1e2127",
        panel: "#282c34",
        ink: "#e6efff",
        muted: "#828791",
        border: "#5c6370",
        primary: "#61afef",
        accent: "#56b6c2",
        success: "#98c379",
        warn: "#d19a66",
        danger: "#e06c75",
        magenta: "#c678dd",
      },
    },
  },
  plugins: [],
};

export default config;
