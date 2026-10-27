// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { checkRepository } from './doc-dependency-versions.mjs';
try {
  const result = checkRepository();
  console.log(`Documentation versions OK: ${result.documents} documents, ${result.crates} crate versions, and tutorial recipes.`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
