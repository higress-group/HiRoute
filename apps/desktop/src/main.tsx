import React from 'react';
import { ConsoleRoot } from './host/ConsoleRoot';
import { createRoot } from 'react-dom/client';
import { DesktopApp } from './product/DesktopApp';
import { RenderFailureBoundary } from './ui/RenderFailureBoundary';
import './occami/styles.css';

createRoot(document.getElementById('root')!).render(
  <RenderFailureBoundary>
    <ConsoleRoot><DesktopApp /></ConsoleRoot>
  </RenderFailureBoundary>,
);
