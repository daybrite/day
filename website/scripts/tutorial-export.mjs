// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
import { readRecipe, resolvePlan } from '../src/lib/tutorials.mjs';
const recipe = readRecipe();
console.log(JSON.stringify({ recipe, steps: resolvePlan(recipe, process.argv[2] ?? 'macos') }));
