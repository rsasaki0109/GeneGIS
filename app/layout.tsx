import type { Metadata } from "next";
import "./globals.css";

const title = "GeneGIS Playground — verified geospatial workflows";
const description =
  "Turn a natural-language intent into an auditable geospatial workflow, verified result, map, and provenance.";
// Rendered by `genegis demo social-card`; refuses to render unless every check passes.
const socialImage = {
  url: "/og.png",
  width: 1280,
  height: 640,
  alt: "名古屋市の人口密度 as verified 3D population-mesh columns (7/7 checks)",
};

export const metadata: Metadata = {
  metadataBase: new URL("https://genegis-playground.rsasaki0109.chatgpt.site"),
  title,
  description,
  openGraph: { type: "website", siteName: "GeneGIS", title, description, images: [socialImage] },
  twitter: { card: "summary_large_image", title, description, images: [socialImage] },
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="ja">
      <body>{children}</body>
    </html>
  );
}
