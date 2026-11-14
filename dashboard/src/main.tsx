import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { adoptTokenFromUrl } from './utils/bridge'

// Before anything connects: a link like http://host:9000/?token=… sets up
// this browser for a remote bridge.
adoptTokenFromUrl()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
