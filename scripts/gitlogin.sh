#!/bin/bash
GUSER=GssMahadevan
source .env
git remote set-url origin https://$GUSER:$GPAT@github.com/$GUSER/gvthread.git
