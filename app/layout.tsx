import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "GeneGIS Playground — verified geospatial workflows",
  description:
    "Turn a natural-language intent into an auditable geospatial workflow, verified result, map, and provenance.",
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="ja">
      <body>{children}</body>
    </html>
  );
}
