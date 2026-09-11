// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
const guides = {
  'Day-Rise': ['guide-storage', 'Saving app data'],
  'Day-Skies': ['guide-http', 'Making HTTP requests'],
  'Day-Tradr': ['reactivity', 'Updating the UI from state'],
  'Day-News': ['navigation', 'Building app navigation'],
  'Day-Sketch': ['rendering', 'Drawing and rendering'],
  'Day-Games': ['rendering', 'Drawing and rendering'],
};
const screens = {
  controls: ['pieces', 'Composing UI controls'],
  layout: ['layout', 'Arranging controls'],
  grid: ['layout', 'Arranging controls'],
  stack: ['navigation', 'Building app navigation'],
  tabs: ['navigation', 'Building app navigation'],
  network: ['guide-http', 'Making HTTP requests'],
  files: ['guide-storage', 'Saving app data'],
  localization: ['localization', 'Translating an app'],
  resources: ['resources', 'Using app resources'],
  canvas: ['rendering', 'Drawing and rendering'],
  notify: ['guide-notifications', 'Sending notifications'],
};
export function guideFor(app, screen) {
  return (app === 'Day-Showcase' && screens[screen.split('-')[0]]) || guides[app] || ['api-tour', 'Touring the UI API'];
}
