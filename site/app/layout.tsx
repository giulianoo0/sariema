// SPDX-License-Identifier: Apache-2.0

import type { Metadata, Viewport } from "next";
import type { ReactNode } from "react";
import { GeistMono } from "geist/font/mono";
import { GeistSans } from "geist/font/sans";
import { RootProvider } from "fumadocs-ui/provider/next";
import "./globals.css";

const site = process.env.SITE_URL;

export const metadata: Metadata = {
  title: { default: "Sariema", template: "%s · Sariema" },
  description:
    "An open-source game engine in Rust on wgpu, built to be embedded in apps and to run games shipped as data packs.",
  icons: {
    icon: [
      { url: "/brand/favicon.svg", type: "image/svg+xml" },
      { url: "/brand/favicon-32.png", sizes: "32x32", type: "image/png" },
    ],
  },
  // A social card needs the site's absolute address, known only at deploy.
  ...(site && {
    metadataBase: new URL(site),
    openGraph: { images: "/brand/og.png", siteName: "Sariema" },
    twitter: { card: "summary_large_image" },
  }),
};

export const viewport: Viewport = {
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#fbf6ec" },
    { media: "(prefers-color-scheme: dark)", color: "#17130f" },
  ],
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={`${GeistSans.variable} ${GeistMono.variable}`} suppressHydrationWarning>
      <body className="flex min-h-screen flex-col">
        <RootProvider
          theme={{ attribute: ["class", "data-theme"], hotKey: false }}
          search={{ options: { type: "static" } }}
        >
          {children}
        </RootProvider>
      </body>
    </html>
  );
}
