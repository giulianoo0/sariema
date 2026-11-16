// SPDX-License-Identifier: Apache-2.0

import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { Card, Cards } from "fumadocs-ui/components/card";
import { DocsBody, DocsDescription, DocsPage, DocsTitle, EditOnGitHub } from "fumadocs-ui/layouts/docs/page";
import defaultMdxComponents from "fumadocs-ui/mdx";
import { repo } from "@/lib/markdown.mjs";
import { docsSections, source } from "@/lib/source";

type Props = { params: Promise<{ slug?: string[] }> };

export default async function Page({ params }: Props) {
  const { slug } = await params;
  if (!slug?.length) return <Index />;
  const page = source.getPage(slug);
  if (!page) notFound();
  const Body = page.data.body;
  return (
    <DocsPage toc={page.data.toc} tableOfContent={{ style: "clerk" }}>
      <DocsTitle>{page.data.title}</DocsTitle>
      <DocsBody>
        <Body components={defaultMdxComponents} />
      </DocsBody>
      <EditOnGitHub href={`${repo}/edit/main/docs/${page.path}`} />
    </DocsPage>
  );
}

function Index() {
  return (
    <DocsPage>
      <DocsTitle>Documentation</DocsTitle>
      <DocsDescription>
        Every page here is a file in the repository&apos;s docs/ folder, read as it is.
      </DocsDescription>
      <DocsBody>
        {docsSections().map((section) => (
          <section key={section.title}>
            <h2>{section.title}</h2>
            <Cards>
              {section.docs.map(({ page, label }) => (
                <Card key={page.url} href={page.url} title={label}>
                  {page.data.description}
                </Card>
              ))}
            </Cards>
          </section>
        ))}
      </DocsBody>
    </DocsPage>
  );
}

export function generateStaticParams() {
  return [{ slug: [] }, ...source.generateParams()];
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const { slug } = await params;
  if (!slug?.length) return { title: "Documentation" };
  const page = source.getPage(slug);
  if (!page) notFound();
  return { title: page.data.title, description: page.data.description };
}
