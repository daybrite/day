// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { recipeHash } from '../../lib/tutorials.mjs';
import { tutorialRecipe as recipe } from '../../lib/tutorial-data';
export function GET() {
  return new Response(JSON.stringify({ recipe, sha256: recipeHash(recipe) }), {
    headers: { 'Content-Type': 'application/json; charset=utf-8' },
  });
}
