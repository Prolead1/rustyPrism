echo "Generating $2 FIX messages to $1..."
tickers=("AAPL" "GOOGL" "MSFT" "AMZN" "FB")  # List of tickers
sides=("1" "2")  # Buy (1) and Sell (2) sides

for i in $(seq 1 $2); do
    ticker=${tickers[$((($i - 1) % ${#tickers[@]}))]}  # Select ticker based on modulus division
    side=${sides[$((($i - 1) % ${#sides[@]}))]}  # Select side based on modulus division
    price=$((RANDOM % 100 + 1))  # Generate random price between 1 and 1000
    quantity=$((RANDOM % 100 + 1))  # Generate random quantity between 1 and 100
    quantity=$((quantity * 100))  # Make quantity a multiple of 100

    message="8=FIX.4.4|9=0|35=D|49=SENDER|56=TARGET|34=$i|55=$ticker|54=$side|44=$price|38=$quantity|10=000|"
    bodyLength=$((${#message} - 7))  # Calculate BodyLength value
    checksum=$(printf "%03d" $(($(printf '%d' "'$message" | awk '{for(i=1;i<=NF;i++)s+=$i}END{print s%256}') % 256)))  # Calculate CheckSum value

    echo "8=FIX.4.4|9=$bodyLength|35=D|49=SENDER|56=TARGET|34=$i|55=$ticker|54=$side|44=$price|38=$quantity|10=$checksum|"
done > $1
echo "Done."