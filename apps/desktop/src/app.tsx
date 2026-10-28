import { Glyph } from "./ui/glyph.tsx";
import "./app.css";

export function App() {
  return (
    <div className="window">
      <header className="titlebar" data-tauri-drag-region />
      <main className="welcome">
        <Glyph size={56} />
        <h1>TableUltra</h1>
        <p>Connect to a PostgreSQL database to begin.</p>
      </main>
    </div>
  );
}
