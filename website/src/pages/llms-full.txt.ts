// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import data from '../lib/llm-data';
export function GET() {
  return new Response(data.full, { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
}
