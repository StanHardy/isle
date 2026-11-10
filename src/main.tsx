import { createRoot } from "react-dom/client";
import IslandRoot from "./island/IslandRoot";
import { frontendLog } from "./shared/api";
import "./styles/island.css";

// Windowed app has no console — surface FE crashes in the Rust debug log.
window.addEventListener("error", (e) => {
  frontendLog(`error: ${e.message} @ ${e.filename}:${e.lineno}:${e.colno}\n${e.error?.stack ?? ""}`);
});
window.addEventListener("unhandledrejection", (e) => {
  frontendLog(`unhandledrejection: ${String(e.reason)}`);
});

createRoot(document.getElementById("root")!).render(<IslandRoot />);
