#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

TIMEOUT=180
COMPOSE_FILE="${1:-ci/docker-compose.yml}"

echo -e "${BLUE}===========================================${NC}"
echo -e "${BLUE}Waiting for Test Services to be Ready${NC}"
echo -e "${BLUE}===========================================${NC}"
echo ""

# Function to wait for a service with healthcheck
wait_for_service() {
    local service=$1
    local description=$2
    local start_time=$(date +%s)
    
    echo -e "${YELLOW}⏳ Waiting for $description...${NC}"
    
    while true; do
        current_time=$(date +%s)
        elapsed=$((current_time - start_time))
        
        if [ $elapsed -gt $TIMEOUT ]; then
            echo -e "${RED}❌ Timeout waiting for $description (${TIMEOUT}s)${NC}"
            echo -e "${YELLOW}📋 Container logs:${NC}"
            docker compose -f $COMPOSE_FILE logs --tail=50 $service
            exit 1
        fi
        
        # Check if container is healthy
        health=$(docker compose -f $COMPOSE_FILE ps --format json $service 2>/dev/null | jq -r '.[0].Health // "unknown"')
        
        if [ "$health" = "healthy" ]; then
            echo -e "${GREEN}✅ $description is ready (${elapsed}s)${NC}"
            return 0
        elif [ "$health" = "unhealthy" ]; then
            echo -e "${RED}❌ $description is unhealthy${NC}"
            echo -e "${YELLOW}📋 Container logs:${NC}"
            docker compose -f $COMPOSE_FILE logs --tail=50 $service
            exit 1
        fi
        
        # Show progress
        echo -ne "${YELLOW}⏱️  Waiting for $description... ${elapsed}s / ${TIMEOUT}s\r${NC}"
        sleep 2
    done
}

# Function to wait for specific log message
wait_for_log() {
    local service=$1
    local pattern=$2
    local description=$3
    local start_time=$(date +%s)
    
    echo -e "${YELLOW}⏳ Waiting for $description...${NC}"
    
    while true; do
        current_time=$(date +%s)
        elapsed=$((current_time - start_time))
        
        if [ $elapsed -gt $TIMEOUT ]; then
            echo -e "${RED}❌ Timeout waiting for $description (${TIMEOUT}s)${NC}"
            echo -e "${YELLOW}📋 Container logs:${NC}"
            docker compose -f $COMPOSE_FILE logs --tail=100 $service
            exit 1
        fi
        
        if docker compose -f $COMPOSE_FILE logs $service 2>&1 | grep -q "$pattern"; then
            echo -e "${GREEN}✅ $description detected (${elapsed}s)${NC}"
            return 0
        fi
        
        echo -ne "${YELLOW}⏱️  Waiting for $description... ${elapsed}s / ${TIMEOUT}s\r${NC}"
        sleep 2
    done
}

# Function to mine initial blocks
mine_initial_blocks() {
    echo -e "${YELLOW}⛏️  Mining initial blocks for Bitcoin...${NC}"
    
    # Create wallet
    docker compose -f $COMPOSE_FILE exec -T bitcoind \
        bitcoin-cli -regtest -rpcuser=user -rpcpassword=password \
        createwallet "miner" 2>/dev/null || echo "Wallet already exists"
    
    # Get new address
    local address=$(docker compose -f $COMPOSE_FILE exec -T bitcoind \
        bitcoin-cli -regtest -rpcuser=user -rpcpassword=password \
        -rpcwallet=miner getnewaddress)
    
    # Mine 103 blocks (need 101+ for coinbase maturity)
    echo -e "${YELLOW}   Mining 103 blocks to address: $address${NC}"
    docker compose -f $COMPOSE_FILE exec -T bitcoind \
        bitcoin-cli -regtest -rpcuser=user -rpcpassword=password \
        -rpcwallet=miner generatetoaddress 103 "$address" > /dev/null
    
    echo -e "${GREEN}✅ Mined 103 blocks${NC}"
}

# Function to verify service is responding
verify_service_api() {
    local url=$1
    local description=$2
    
    echo -e "${YELLOW}🔍 Verifying $description API...${NC}"
    
    if curl -s -f "$url" > /dev/null; then
        echo -e "${GREEN}✅ $description API is responding${NC}"
        return 0
    else
        echo -e "${RED}❌ $description API is not responding${NC}"
        exit 1
    fi
}

# Main wait sequence
echo -e "${BLUE}Step 1: Bitcoin Core${NC}"
wait_for_service "bitcoind" "Bitcoin Core"

echo ""
echo -e "${BLUE}Step 2: Initial Blockchain Setup${NC}"
mine_initial_blocks

echo ""
echo -e "${BLUE}Step 3: Electrs Indexer${NC}"
wait_for_service "electrs" "Electrs"
wait_for_log "electrs" "finished full compaction" "Electrs initial indexing"

echo ""
echo -e "${BLUE}Step 4: F1r3node${NC}"
wait_for_service "f1r3node" "F1r3node"
wait_for_log "f1r3node" "Making a transition to Running" "F1r3node running state"

echo ""
echo -e "${BLUE}Step 5: API Verification${NC}"
verify_service_api "http://localhost:3002/api/blocks/tip/height" "Electrs (Esplora)"
verify_service_api "http://localhost:40403/api/status" "F1r3node"

echo ""
echo -e "${GREEN}===========================================${NC}"
echo -e "${GREEN}✅ All Services Ready for Testing${NC}"
echo -e "${GREEN}===========================================${NC}"
echo ""
echo -e "${BLUE}Service Endpoints:${NC}"
echo -e "  Bitcoin RPC:  http://localhost:18443"
echo -e "  Esplora API:  http://localhost:3002"
echo -e "  F1r3node HTTP: http://localhost:40403"
echo -e "  F1r3node gRPC: localhost:40401"
echo ""

