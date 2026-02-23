import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        canvas: "#f3f5f7",
        panel: "#ffffff",
        ink: "#0f1720",
        muted: "#5b6673",
        accent: "#0f766e",
        warn: "#b45309",
        danger: "#b91c1c",
      },
    },
  },
  plugins: [],
};

export default config;
