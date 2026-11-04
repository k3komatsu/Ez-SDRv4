# Orchestrator rulings on test-plan-s1 open questions (2026-11-04)
- Q1: stream names are unique within one connect call across both directions (rx and tx).
- Q2: capture (or any alias read) on a radio with no receive stream raises Error, as EA-16's alias rule does; the message tells the user to give connect(rx=...).
- Q3: radio= selects the one radio the numbered rx/tx maps configure (default "radio"); configuring several radios in one connect is left to channel names (stage 1b) or 8b.
- Q4: Connected.streams (name -> number) in the connect reply is accepted; note 47 records it.
