// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import data from '../lib/llm-data';
export function getStaticPaths() {
  return data.guides.map((guide) => ({ params: { guide: guide.route.slice(1) }, props: { key: guide.key } }));
}
export function GET({ props }: { props: { key: string } }) {
  const guide = data.guides.find((guide) => guide.key === props.key)!;
  return new Response(guide.markdown, { headers: { 'Content-Type': 'text/markdown; charset=utf-8' } });
}
