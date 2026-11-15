import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { OrbitBench } from "./OrbitBench";
import "./styles.css";

// Bench mode: #orbit-bench in the URL, or VITE_BENCH=orbit in the
// environment (lets the dev app open straight into the perf bench).
const bench =
  window.location.hash === "#orbit-bench" ||
  (import.meta.env.VITE_BENCH as string | undefined) === "orbit";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {bench ? <OrbitBench /> : <App />}
  </React.StrictMode>,
);
