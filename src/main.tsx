import { createRoot } from "react-dom/client";
import IslandRoot from "./island/IslandRoot";
import "./styles/island.css";

createRoot(document.getElementById("root")!).render(<IslandRoot />);
